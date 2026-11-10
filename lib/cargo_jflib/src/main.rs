//! `cargo jflib` -- what Cargo does not do for a JFLib app: a new project, its test scripts, the
//! bundle for the browser.  `cargo install cargo-jflib`, then:
//!
//! ```text
//! cargo jflib new [--std] NAME         a new app NAME from the template (a window); --std: with std
//! cargo jflib test [-p PKG] [--release] [--target TRIPLE] [--features F,..] [--no-default-features] [--dir DIR]
//!                                      build, then run every tests/*.uis (DIR/*.uis) with --script
//! cargo jflib watch [-p PKG] [-- ARGS]  hot reload: built again and restarted in its session on a save
//! cargo jflib web [-p PKG] [--release] [--modes scalar,simd,threads] [--serve [PORT]]
//!                                      the app for the browser in target/jflib-web/NAME/
//! cargo jflib android [-p PKG] [--release] [--abi arm64-v8a|x86_64] [--install]
//!                                      the app as an APK in target/jflib-android/
//! cargo jflib mac [-p PKG] [--release] [--run [-- ARGS]]
//!                                      the app as NAME.app in target/jflib-mac/ (on a Mac)
//! cargo jflib package [-p PKG] [--styles winui,kde] [--formats setup,deb,..|all] [--no-win] [--no-deb]
//!                     [--sign] [--repo DIR] [--update DIR [--update-url URL] [--update-key FILE]]
//!                                      setup, MSIX, .deb, AppImage, RPM, Flatpak in target/jflib-package/
//! cargo jflib keygen FILE              a key for the update channel and its public key
//! ```
//!
//! It calls Cargo for the builds and needs nothing else; the browser with threads needs the
//! Rust sources (`rustup component add rust-src`) for a core library with atomics, Android the
//! SDK (build-tools, platform 36), the NDK and a JDK.

mod android;
mod cargo;
mod debfile;
mod deflate;
mod json;
mod linux;
mod linuxpkg;
mod lz4;
mod mac;
mod msix;
mod new;
mod notices;
mod package;
mod pkgmeta;
mod png;
mod sha256;
mod sign;
mod update;
mod watch;
mod squashfs;
mod test;
mod web;
mod zip;

use std::process::ExitCode;

const HELP: &str = "cargo jflib -- a JFLib app's new project, tests, browser bundle, APK, Mac app and packages

USAGE:
    cargo jflib new [--std] NAME
        a new app NAME from the template: a window, its tests, Linux and Windows settings;
        --std: an app with the standard library (jflib's feature std), so it may use std crates
    cargo jflib test [-p PKG] [--release] [--target TRIPLE] [--features F,..] [--no-default-features] [--dir DIR]
        build, then run every tests/*.uis of the package with --script; --features (repeated or
        F,G) and --no-default-features go to the build, --dir DIR runs DIR/*.uis instead (the
        package's folder is the base of a relative DIR)
    cargo jflib watch [-p PKG] [-- ARGS]
        hot reload: a development build, started in its session (target/jflib-watch/NAME); each
        save of the project's files builds again and restarts it, the input replayed so the app
        comes back where it was; a failed build keeps the running version (the error in its
        inspector); ARGS go to every start; Ctrl+C ends watch and the app
    cargo jflib web [-p PKG] [--release] [--modes scalar,simd,threads] [--serve [PORT]]
        the app for the browser: target/jflib-web/NAME/ (index.html, the modules, the host);
        --serve then serves it on 127.0.0.1 (default port 8766) with the headers threads need
    cargo jflib android [-p PKG] [--release] [--abi arm64-v8a|x86_64] [--install]
        the app as an APK: target/jflib-android/NAME-ABI-debug.apk (--release: signed with the key
        of jfgui_ANDROID_KEYSTORE); needs the Android SDK and NDK and a JDK; --install: adb install
    cargo jflib mac [-p PKG] [--release] [--run [-- ARGS]]
        the app as NAME.app: target/jflib-mac/NAME.app (Info.plist from PACKAGE, the icon from res/,
        signed ad hoc; with `sandbox` in PERMISSIONS the App Sandbox); on a Mac; --run starts it
    cargo jflib package [-p PKG] [--styles winui,kde] [--formats setup,deb,..|all] [--no-win] [--no-deb]
        the app with every style built in, the look picked at installation, in target/jflib-package/:
        setup (NAME_Setup.exe) and deb by default, also msix (MakeAppx), appimage (one per style),
        rpm (the .spec; the .rpm with rpmbuild) and flatpak (the manifest; the bundle with
        flatpak-builder; on Windows through WSL); the styles from --styles or PACKAGE styles=,
        default winui; the rest of PACKAGE as in README.md of cargo-jflib.  Signed with what the
        environment names (jfgui_SIGN_PFX / jfgui_SIGN_THUMBPRINT: Authenticode, jfgui_GPG_KEY:
        AppImages, apt repository, Flatpak); --sign: every signature required
        --repo DIR: a flat apt repository with the .deb; --update DIR: a signed update channel
        per style (the key from --update-key, jfgui_UPDATE_KEY_FILE or jfgui_UPDATE_KEY)
    cargo jflib keygen FILE [-p PKG]
        a new update key in FILE (never overwritten) and its public key for update::Config
    cargo jflib --version";

/// the options every command shares
#[derive(Default, Debug)]
pub(crate) struct Opts {
    pub package: Option<String>,
    pub release: bool,
    pub target: Option<String>,
    pub modes: Option<String>,
    pub serve: Option<u16>,
    /// (new) an app with std: jflib's feature std
    pub std: bool,
    /// (test) the package's features to build with (--features, Cargo's form: "a,b" or "a b",
    /// repeated), and --no-default-features
    pub features: Vec<String>,
    pub no_default_features: bool,
    /// (test) the folder of the scripts instead of tests/
    pub dir: Option<String>,
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
            "--std" => o.std = true,
            "--target" => o.target = Some(value(&mut i, "--target")?),
            "--modes" => o.modes = Some(value(&mut i, "--modes")?),
            "--features" | "-F" => {
                let v = value(&mut i, "--features")?;
                o.features.extend(v.split([',', ' ']).filter(|f| !f.is_empty()).map(String::from));
            }
            "--no-default-features" => o.no_default_features = true,
            "--dir" => o.dir = Some(value(&mut i, "--dir")?),
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

/// the options only `test` uses: refused by the other commands (before a `--`, whose rest goes to
/// the app) rather than left without effect
fn test_only(args: &[String]) -> Result<(), String> {
    let Some(cmd) = args.first().filter(|c| c.as_str() != "test") else { return Ok(()) };
    let mut own = args[1..].iter().take_while(|a| a.as_str() != "--");
    match own.find(|a| matches!(a.as_str(), "--features" | "-F" | "--no-default-features" | "--dir")) {
        Some(a) => Err(format!("{a}: only cargo jflib test takes it, not {cmd}")),
        None => Ok(()),
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("jflib") {
        args.remove(0); // `cargo jflib X` runs `cargo-jflib jflib X`
    }
    if let Err(e) = test_only(&args) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new::run(&args[1..]),
        Some("test") => test::run(&args[1..]),
        Some("web") => web::run(&args[1..]),
        Some("watch") => watch::run(&args[1..]),
        Some("android") => android::run_cmd(&args[1..]),
        Some("mac") => mac::run_cmd(&args[1..]),
        Some("package") => package::run(&args[1..]),
        Some("keygen") => update::keygen(&args[1..]),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_options() {
        let o = opts(&args(&["--features", "engine,mock", "-F", "extra", "--no-default-features", "--dir", "tests/engine", "--release"])).unwrap();
        assert_eq!(o.features, ["engine", "mock", "extra"]);
        assert!(o.no_default_features && o.release);
        assert_eq!(o.dir.as_deref(), Some("tests/engine"));
        let o = opts(&args(&["--features", "a b", "-p", "app"])).unwrap();
        assert_eq!((o.features.as_slice(), o.package.as_deref(), o.no_default_features, o.dir), (&["a".to_string(), "b".to_string()][..], Some("app"), false, None));
        assert!(opts(&args(&["--features"])).is_err() && opts(&args(&["--dir"])).is_err());
        assert!(opts(&args(&[])).unwrap().features.is_empty());
        // only test takes them; the app's arguments after -- are not looked at
        assert!(test_only(&args(&["test", "--features", "x", "--dir", "d"])).is_ok());
        assert!(test_only(&args(&["web", "--no-default-features"])).is_err() && test_only(&args(&["package", "-F", "x"])).is_err());
        assert!(test_only(&args(&["watch", "--", "--dir", "x"])).is_ok() && test_only(&args(&[])).is_ok());
    }
}
