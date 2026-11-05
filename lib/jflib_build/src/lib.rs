//! jflib-build -- what a JFLib app's build script hands to `ui::main!` and `rt::main!`
//!
//! ```text
//! // build.rs of a program with a window
//! fn main() {
//!     jflib_build::app();
//! }
//! ```
//!
//! [`console`] does the same for a command-line program (on Windows the console subsystem, so a
//! shell waits for it and shows its output).  Both read what lies next to the project's
//! Cargo.toml, all of it optional:
//!
//! | File | What the program gets |
//! | --- | --- |
//! | `PERMISSIONS` | what it may do (JFLib's docs/PERMISSIONS.md), without the file everything; an error names the line |
//! | `PACKAGE` | `display=`, `app_id=`, `categories=`, `filetypes=`, `schemes=` for `--register`; a `version=` must be Cargo's |
//! | `assets/` | the files of `app://`, packed as JFA1 with LZ4 |
//! | `res/app.ico` | the program's icon on Windows |
//!
//! The version of the program (`rt::app_version!()`) is the package's version in Cargo.toml.
//!
//! It also gives the linker what JFLib's own runtime needs: its entry point and no C runtime, on
//! Windows the subsystem; on Linux one static file or, with `--cfg jfgui_glibc`, a program
//! against glibc (the libraries to link against are made here, the build machine needs no glibc);
//! for the browser the exports of the wasm module.  What a build script cannot set, the linker
//! and the relocation model, comes from the project's `.cargo/config.toml`: the template in the
//! crate `jflib` has it, an error here names the line that is missing.
#![forbid(unsafe_code)]

mod assets;
mod icon;
mod package;
mod permissions;
mod target;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// what kind of program the package builds
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// a window: the Windows subsystem
    Gui,
    /// a command-line program: the console subsystem
    Console,
}

/// The build script of a program with a window.
pub fn app() {
    Build::app().run()
}

/// The build script of a command-line program (Windows: console subsystem).
pub fn console() {
    Build::console().run()
}

/// A build script with more to say than [`app`] and [`console`]:
///
/// ```text
/// // build.rs of the command-line program bc-server of the app BackupCraft, in apps/BackupCraft/server
/// fn main() {
///     jflib_build::Build::console().part_of("BackupCraft", "..").run();
/// }
/// ```
pub struct Build {
    kind: Kind,
    app: Option<(String, PathBuf)>,
}

impl Build {
    /// a program with a window
    pub fn app() -> Build {
        Build { kind: Kind::Gui, app: None }
    }

    /// a command-line program (Windows: console subsystem)
    pub fn console() -> Build {
        Build { kind: Kind::Console, app: None }
    }

    /// One of several programs of the app `name`, whose `PERMISSIONS`, `PACKAGE`, `assets/` and
    /// `res/` lie in `dir` (relative to this package): the program gets the app's name and files.
    /// A `PERMISSIONS.<this package's name>` there wins over the app's `PERMISSIONS`.
    pub fn part_of(mut self, name: &str, dir: &str) -> Build {
        self.app = Some((name.to_string(), PathBuf::from(dir)));
        self
    }

    /// print what Cargo hands to the compiler and the linker
    pub fn run(self) {
        run(self.kind, self.app)
    }
}

fn var(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} is not set: jflib_build runs in a build script"))
}

/// Cargo's instructions, printed only when nothing failed
struct Out {
    lines: Vec<String>,
    errors: Vec<String>,
}

impl Out {
    fn env(&mut self, key: &str, value: &Path) {
        self.lines.push(format!("cargo::rustc-env={key}={}", value.display()));
    }

    fn cfg(&mut self, cfg: &str) {
        self.lines.push(format!("cargo::rustc-cfg={cfg}"));
    }
}

fn run(kind: Kind, app: Option<(String, PathBuf)>) {
    let package = PathBuf::from(var("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(var("OUT_DIR"));
    let package_name = var("CARGO_PKG_NAME");
    let version = var("CARGO_PKG_VERSION");
    let (name, dir) = match app {
        Some((name, dir)) => (name, package.join(dir)),
        None => (package_name.clone(), package.clone()),
    };
    let own = dir.join(format!("PERMISSIONS.{package_name}"));
    let manifest = if own.is_file() { own } else { dir.join("PERMISSIONS") };
    let mut out = Out { lines: Vec::new(), errors: Vec::new() };
    for c in ["jfgui_manifest", "jfgui_package", "jfgui_assets", "jfgui_glibc", "jfgui_dev", "web_threads"] {
        out.lines.push(format!("cargo::rustc-check-cfg=cfg({c})"));
    }
    // A path that does not exist makes Cargo run this script and compile the program again on
    // every build: only what is there is watched.  A file added later counts after
    // `cargo clean -p NAME` (README.md of the crate jflib).
    let mut watched: Vec<PathBuf> =
        [manifest.clone(), dir.join("PACKAGE"), dir.join("assets"), dir.join("res/app.ico")].into_iter().filter(|p| p.exists()).collect();
    if watched.is_empty() {
        // without a single rerun-if-changed Cargo would watch the whole package, target/ too
        watched.push(package.join("Cargo.toml"));
    }
    for p in watched {
        out.lines.push(format!("cargo::rerun-if-changed={}", p.display()));
    }
    out.lines.push(format!("cargo::rustc-env=JFGUI_APP={name}"));
    out.lines.push(format!("cargo::rustc-env=JFGUI_VERSION={version}"));

    if let Some(text) = read(&manifest, &mut out) {
        match permissions::check(&text) {
            Ok(()) => {
                out.env("JFGUI_PERMISSIONS_FILE", &manifest);
                out.cfg("jfgui_manifest");
            }
            Err((line, msg)) => {
                let file = manifest.file_name().map_or("PERMISSIONS".into(), |f| f.to_string_lossy());
                out.errors.push(format!("{file}:{line}: {msg}"))
            }
        }
    }
    if let Some(text) = read(&dir.join("PACKAGE"), &mut out) {
        match package::boot_text(&text, &name, &version) {
            Ok(boot) => write(&out_dir.join("package.txt"), boot.as_bytes(), "JFGUI_PACKAGE_FILE", "jfgui_package", &mut out),
            Err(e) => out.errors.push(e),
        }
    }
    let assets = dir.join("assets");
    if assets.is_dir() {
        match assets::pack(&assets) {
            Ok(bundle) => write(&out_dir.join("app.jfa"), &bundle, "JFGUI_ASSETS", "jfgui_assets", &mut out),
            Err(e) => out.errors.push(format!("assets: {e}")),
        }
    }
    match target::Target::from_env().and_then(|t| t.link(kind, &dir, &out_dir)) {
        Ok(args) => out.lines.extend(args),
        Err(e) => out.errors.push(e),
    }
    if out.errors.is_empty() {
        for line in &out.lines {
            println!("{line}");
        }
    } else {
        for e in &out.errors {
            println!("cargo::error={e}");
        }
    }
}

/// a file next to Cargo.toml as text, None without it
fn read(path: &Path, out: &mut Out) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) => {
            out.errors.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

/// a file of OUT_DIR the program builds in through include_bytes! / include_str!
fn write(path: &Path, data: &[u8], key: &str, cfg: &str, out: &mut Out) {
    match fs::write(path, data) {
        Ok(()) => {
            out.env(key, path);
            out.cfg(cfg);
        }
        Err(e) => out.errors.push(format!("{}: {e}", path.display())),
    }
}
