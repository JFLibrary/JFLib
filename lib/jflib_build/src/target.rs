// The linker arguments JFLib's runtime needs on each target: build_rs.py's native_link_args, the
// glibc stubs of its section "glibc" and the wasm exports of its web build.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Kind;

/// what Cargo tells a build script about the target
pub(crate) struct Target {
    triple: String,
    os: String,
    arch: String,
    env: String,
    features: Vec<String>,
    /// --cfg jfgui_glibc in the rustflags (templates up to jflib 0.2.0, build_rs.py)
    glibc: bool,
    rustflags: String,
    /// a program with std (jflib's feature std, DEP_JFLIB_STD): the C runtime's entry and libraries
    std: bool,
}

// section "glibc" of build_rs.py: the libraries to link against, one "ret" per name, every symbol
// at the oldest version of the architecture; at run time the system's libraries are used
const GLIBC_STUBS: [(&str, &[&str]); 3] = [
    ("libpthread.so.0", &["pthread_attr_destroy", "pthread_attr_init", "pthread_attr_setstack", "pthread_create", "pthread_join"]),
    ("libdl.so.2", &["dlopen", "dlsym", "dlclose", "dlerror"]),
    ("libc.so.6", &["__libc_start_main", "__cxa_atexit", "__errno_location", "exit", "gnu_get_libc_version", "pthread_self"]),
];

// ARM64 asks libc.so.6 for one more: getauxval (rt::os::page_size, AT_PAGESZ; glibc has it since 2.16)
const GLIBC_STUBS_AARCH64: &[&str] = &["getauxval"];

fn glibc_abi(arch: &str) -> Option<(&'static str, &'static str)> {
    match arch {
        "x86_64" => Some(("/lib64/ld-linux-x86-64.so.2", "GLIBC_2.2.5")),
        "aarch64" => Some(("/lib/ld-linux-aarch64.so.1", "GLIBC_2.17")),
        _ => None,
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let shown = format!("{cmd:?}");
    let out = cmd.output().map_err(|e| format!("{shown}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{shown}: {}", String::from_utf8_lossy(&out.stderr).trim().replace('\n', " | ")));
    }
    Ok(())
}

impl Target {
    pub(crate) fn from_env() -> Result<Target, String> {
        let get = |k: &str| env::var(k).unwrap_or_default();
        Ok(Target {
            triple: get("TARGET"),
            os: get("CARGO_CFG_TARGET_OS"),
            arch: get("CARGO_CFG_TARGET_ARCH"),
            env: get("CARGO_CFG_TARGET_ENV"),
            features: get("CARGO_CFG_TARGET_FEATURE").split(',').map(str::to_string).collect(),
            glibc: env::var_os("CARGO_CFG_JFGUI_GLIBC").is_some(),
            rustflags: get("CARGO_ENCODED_RUSTFLAGS").replace('\x1f', " "),
            std: crate::std_program(),
        })
    }

    fn flag(&self, f: &str) -> bool {
        self.rustflags.contains(f)
    }

    /// the cargo:: lines of the linker arguments
    pub(crate) fn link(&self, kind: Kind, dir: &Path, out_dir: &Path) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        if self.std && self.os == "windows" && !self.features.iter().any(|f| f == "crt-static") {
            lines.push(format!(
                "cargo::warning=a program with std imports VCRUNTIME140.dll, which not every Windows has: -C target-feature=+crt-static \
                 in [target.{}] rustflags of .cargo/config.toml links the C runtime in (cargo jflib new --std writes it)",
                self.triple
            ));
        }
        let args = match self.os.as_str() {
            "windows" => self.windows(kind, dir, out_dir)?,
            "linux" if self.env == "gnu" => self.linux(out_dir)?,
            "android" => return Ok(Vec::new()), // the APK's library: tools/android.py of JFLib (cargo jflib, later)
            // Apple's linker as rustc calls it: libSystem, its start calls plat's main (docs/APPLE.md)
            "macos" | "ios" => return Ok(Vec::new()),
            _ if self.arch == "wasm32" => return Ok(self.wasm()),
            _ => return Err(format!("{}: JFLib builds for Windows, Linux, macOS, iOS, Android and the browser", self.triple)),
        };
        lines.extend(args.into_iter().map(|a| format!("cargo::rustc-link-arg-bins={a}")));
        Ok(lines)
    }

    fn windows(&self, kind: Kind, dir: &Path, out_dir: &Path) -> Result<Vec<String>, String> {
        if self.env != "msvc" {
            return Err(format!("{}: JFLib builds Windows programs for the msvc targets (x86_64-pc-windows-msvc)", self.triple));
        }
        let subsystem = if kind == Kind::Gui { "WINDOWS" } else { "CONSOLE" };
        let map = format!("/MAP:{}", out_dir.join("program.map").display());
        // with std the C runtime's mainCRTStartup, which calls main (also in the Windows subsystem,
        // as #![windows_subsystem] does it), and its libraries
        let mut args: Vec<String> = if self.std {
            ["/ENTRY:mainCRTStartup", &format!("/SUBSYSTEM:{subsystem}"), &map].map(String::from).to_vec()
        } else {
            ["/ENTRY:mainCRTStartup", &format!("/SUBSYSTEM:{subsystem}"), "/NODEFAULTLIB", "/DEBUG:NONE", &map].map(String::from).to_vec()
        };
        let ico = dir.join("res").join("app.ico");
        if ico.is_file() {
            let data = fs::read(&ico).map_err(|e| format!("{}: {e}", ico.display()))?;
            let res = out_dir.join("app.res");
            fs::write(&res, crate::icon::res(&data)?).map_err(|e| format!("{}: {e}", res.display()))?;
            args.push(res.display().to_string());
        }
        Ok(args)
    }

    /// a program against glibc (the default) or, with -C target-feature=+crt-static, one static
    /// file; rt, plat, gpu and ui make the same choice in their build scripts.  The linker is cc
    /// (Rust's default on Linux, so plain `cargo run` needs no flags) or lld (`-C
    /// linker-flavor=ld.lld`, as build_rs.py and `cargo jflib package` link)
    fn linux(&self, out_dir: &Path) -> Result<Vec<String>, String> {
        let triple = &self.triple;
        if self.std {
            return self.linux_std(out_dir);
        }
        let crt_static = self.features.iter().any(|f| f == "crt-static");
        if self.glibc && crt_static {
            return Err(format!("{triple}: --cfg jfgui_glibc links against glibc, -C target-feature=+crt-static makes it static: remove one"));
        }
        let glibc = !crt_static;
        // JFLib's own start does not relocate the program, so it is linked to a fixed address.
        // Code for any address (Rust's default) links there as well; -C relocation-model=static
        // only saves the indirection through the GOT.
        let pic = !self.flag("relocation-model=static");
        if !glibc && pic {
            return Err(format!(
                "{triple}: one static file (-C target-feature=+crt-static) also needs -C relocation-model=static in \
                 [target.{triple}] rustflags of .cargo/config.toml, and cargo build --target {triple}, also on Linux \
                 (the flags must not reach the build scripts and proc macros)"
            ));
        }
        let ld = self.flag("linker-flavor=ld");
        // ld takes its arguments as they are, cc the linker's behind -Wl,
        let wl = |a: &str| if ld { a.to_string() } else { format!("-Wl,{}", a.replace(' ', ",")) };
        let mut args = vec!["-nostdlib".to_string()];
        if pic {
            // rustc asks for a position-independent program (-pie) before these arguments: the
            // last one counts, for cc as for ld
            args.push(if ld { "--no-pie" } else { "-no-pie" }.to_string());
        }
        if glibc {
            let (interp, _) = glibc_abi(&self.arch).ok_or_else(|| format!("{triple}: no glibc build for {}", self.arch))?;
            args.push(wl(&format!("--dynamic-linker={interp}")));
        } else {
            args.push("-static".to_string());
        }
        for a in ["-e _start", "--gc-sections", "-z noexecstack"] {
            if ld {
                args.extend(a.split(' ').map(String::from));
            } else {
                args.push(wl(a));
            }
        }
        // The map's path is one argument whatever it holds: a project below "My Projects/" must
        // not have it split at the blank (ld) nor the blank turned into a comma (cc's -Wl,).
        // -Xlinker hands cc's next argument to the linker as it is.
        let map = format!("-Map={}", out_dir.join("program.map").display());
        if ld {
            args.push(map);
        } else {
            args.extend(["-Xlinker".to_string(), map]);
        }
        if glibc {
            args.push(wl("--no-as-needed"));
            args.extend(self.glibc_stubs(&out_dir.join("glibc"))?.iter().map(|p| p.display().to_string()));
        }
        Ok(args)
    }

    /// a program with std: glibc's start files and libraries, through the system's cc as for every
    /// Rust program (built on Linux); the template's flags for JFLib's own runtime do not fit
    fn linux_std(&self, out_dir: &Path) -> Result<Vec<String>, String> {
        let triple = &self.triple;
        if self.flag("linker-flavor=ld") || self.features.iter().any(|f| f == "crt-static") {
            return Err(format!(
                "{triple}: a program with std (jflib's feature std) links against glibc through cc: remove the rustflags of \
                 [target.{triple}] from .cargo/config.toml (rust-lld and crt-static are for JFLib's own runtime)"
            ));
        }
        // through -Xlinker as one argument: behind -Wl, a comma in the path split it
        Ok(vec!["-Xlinker".to_string(), format!("-Map={}", out_dir.join("program.map").display())])
    }

    /// the three libraries a program against glibc links with: rustc assembles the stubs, the lld
    /// that comes with rustup makes them shared libraries with the version of every symbol
    fn glibc_stubs(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        let (_, version) = glibc_abi(&self.arch).ok_or_else(|| format!("{}: no glibc build for {}", self.triple, self.arch))?;
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
        let extra: &[&str] = if self.arch == "aarch64" { GLIBC_STUBS_AARCH64 } else { &[] };
        let mut names: Vec<&str> = GLIBC_STUBS.iter().flat_map(|(_, s)| s.iter().copied()).chain(extra.iter().copied()).collect();
        names.sort();
        let mut asm = String::from("#![no_std]\ncore::arch::global_asm!(\n");
        for n in &names {
            for ln in [format!(".globl {n}"), format!(".type {n},@function"), format!("{n}:")] {
                asm += &format!("    \"{ln}\",\n");
            }
        }
        asm += "    \"ret\",\n);\n";
        let src = dir.join("stub.rs");
        let obj = dir.join("stub.o");
        fs::write(&src, asm).map_err(|e| format!("{}: {e}", src.display()))?;
        run(Command::new(&rustc)
            .args(["--edition", "2024", "--target", &self.triple, "--crate-type", "lib", "--crate-name", "glibc_stub"])
            .args(["-C", "panic=abort", "-C", "codegen-units=1", "--emit=obj", "-o"])
            .arg(&obj)
            .arg(&src))?;
        let lld = rust_lld(&rustc)?;
        let mut out = Vec::new();
        for (lib, symbols) in GLIBC_STUBS {
            let script = dir.join(format!("{lib}.map"));
            let own: &[&str] = if lib == "libc.so.6" { extra } else { &[] };
            let text: String = symbols.iter().chain(own).map(|s| format!("    {s};\n")).collect();
            fs::write(&script, format!("{version} {{\n  global:\n{text}  local: *;\n}};\n")).map_err(|e| format!("{}: {e}", script.display()))?;
            let so = dir.join(lib);
            run(Command::new(&lld)
                .args(["-flavor", "gnu", "-shared", &format!("--soname={lib}")])
                .arg(format!("--version-script={}", script.display()))
                .arg("-o")
                .arg(&so)
                .arg(&obj))?;
            out.push(so);
        }
        Ok(out)
    }

    /// the browser: a module without an entry point that exports its memory; with threads (atomics)
    /// one that imports the shared memory of the page's workers
    fn wasm(&self) -> Vec<String> {
        let mut args = vec!["--no-entry", "--export-memory", "-zstack-size=8388608"];
        if self.features.iter().any(|f| f == "atomics") {
            args.extend(["--import-memory", "--shared-memory", "--max-memory=2147483648", "--initial-memory=33554432", "--export=__stack_pointer"]);
        }
        args.into_iter().map(|a| format!("cargo::rustc-link-arg={a}")).collect()
    }
}

/// the lld of the toolchain (rustup installs it with every target)
fn rust_lld(rustc: &str) -> Result<PathBuf, String> {
    let out = Command::new(rustc).args(["--print", "sysroot"]).output().map_err(|e| format!("{rustc} --print sysroot: {e}"))?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let host = env::var("HOST").unwrap_or_default();
    let exe = if cfg!(windows) { "rust-lld.exe" } else { "rust-lld" };
    let lld = sysroot.join("lib").join("rustlib").join(&host).join("bin").join(exe);
    if lld.is_file() { Ok(lld) } else { Err(format!("{}: not there; it comes with rustup's toolchains", lld.display())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// one static file for Linux, linked by lld (`ld`) or through cc
    fn static_linux(ld: bool) -> Target {
        Target {
            triple: "x86_64-unknown-linux-gnu".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            env: "gnu".into(),
            features: vec!["crt-static".into()],
            glibc: false,
            rustflags: format!("{}-C relocation-model=static", if ld { "-C linker-flavor=ld.lld " } else { "" }),
            std: false,
        }
    }

    #[test]
    fn map_path_with_a_blank_stays_one_argument() {
        let out = Path::new("My Projects").join("app").join("out");
        let map = format!("-Map={}", out.join("program.map").display());
        // lld takes it as it is
        let args = static_linux(true).linux(&out).unwrap();
        assert!(args.contains(&map), "{args:?}");
        assert!(!args.iter().any(|a| a == "-Map=My"), "{args:?}");
        // cc hands it on untouched: no -Wl, whose commas would stand for the blanks
        let args = static_linux(false).linux(&out).unwrap();
        let at = args.iter().position(|a| *a == map).expect("the map argument");
        assert_eq!(args[at - 1], "-Xlinker", "{args:?}");
        assert!(!args.iter().any(|a| a.starts_with("-Wl,-Map")), "{args:?}");
    }

    // with std (glibc through cc): -Xlinker, as -Wl, would split the path at a comma
    #[test]
    fn std_map_path_with_a_comma_stays_one_argument() {
        let out = Path::new("a,b").join("out");
        let t = Target { features: vec![], rustflags: String::new(), glibc: true, std: true, ..static_linux(false) };
        let args = t.linux(&out).unwrap();
        assert_eq!(args, ["-Xlinker".to_string(), format!("-Map={}", out.join("program.map").display())]);
    }

    #[test]
    fn the_other_linker_arguments_are_as_before() {
        let out = Path::new("out");
        let args = static_linux(true).linux(out).unwrap();
        assert_eq!(&args[..7], ["-nostdlib", "-static", "-e", "_start", "--gc-sections", "-z", "noexecstack"]);
        let args = static_linux(false).linux(out).unwrap();
        assert_eq!(&args[..5], ["-nostdlib", "-static", "-Wl,-e,_start", "-Wl,--gc-sections", "-Wl,-z,noexecstack"]);
    }
}
