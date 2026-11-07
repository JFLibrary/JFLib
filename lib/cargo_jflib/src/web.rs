// cargo jflib web: the app for the browser.  Its code is the package's library (src/lib.rs, the
// template's layout); each mode is a wasm module of its own (scalar; SIMD128; with threads: shared
// memory, a core library with atomics built from the Rust sources), next to the host of JFLib's
// web/ (index.html, loader.js, ...) that picks the best one the browser can run (variants.json).

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cargo;

const WASM: &str = "wasm32-unknown-unknown";
/// the browser host, a copy of web/ of JFLib's repository (kept equal there)
const HOST: &[(&str, &[u8])] = &[
    ("loader.js", include_bytes!("../web/loader.js")),
    ("worker.js", include_bytes!("../web/worker.js")),
    ("gpu.js", include_bytes!("../web/gpu.js")),
    ("bridge.js", include_bytes!("../web/bridge.js")),
    ("storage.js", include_bytes!("../web/storage.js")),
    ("secrets.js", include_bytes!("../web/secrets.js")),
    ("handles.js", include_bytes!("../web/handles.js")),
    ("a11y.js", include_bytes!("../web/a11y.js")),
    ("audio.js", include_bytes!("../web/audio.js")),
    ("system.js", include_bytes!("../web/system.js")),
    ("notify.js", include_bytes!("../web/notify.js")),
    ("sensors.js", include_bytes!("../web/sensors.js")),
    ("host.css", include_bytes!("../web/host.css")),
];
const INDEX: &str = include_str!("../web/index.html");

fn rustc() -> String {
    env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string())
}

fn output(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Rust's core, alloc and compiler_builtins for shared memory (atomics), built from rust-src into
/// `dest` once per compiler -> the sysroot; Err without rust-src
fn sysroot(dest: &Path) -> Result<PathBuf, String> {
    let version = output(Command::new(rustc()).args(["--version", "--verbose"]))?;
    let root = PathBuf::from(output(Command::new(rustc()).args(["--print", "sysroot"]))?);
    let src = root.join("lib/rustlib/src/rust/library");
    if !src.join("core/src/lib.rs").is_file() {
        return Err("the browser with threads needs the Rust sources: rustup component add rust-src".to_string());
    }
    let mut h = DefaultHasher::new();
    (version, env!("CARGO_PKG_VERSION")).hash(&mut h);
    let stamp = format!("{:016x}", h.finish());
    let libs = dest.join("lib/rustlib").join(WASM).join("lib");
    let done = ["core", "alloc", "compiler_builtins"].iter().all(|n| libs.join(format!("lib{n}.rlib")).is_file());
    if done && fs::read_to_string(dest.join(".stamp")).ok().as_deref() == Some(&stamp) {
        return Ok(dest.to_path_buf());
    }
    fs::create_dir_all(&libs).map_err(|e| format!("{}: {e}", libs.display()))?;
    let core = libs.join("libcore.rlib");
    let builtins = libs.join("libcompiler_builtins.rlib");
    for (name, file, extra) in [
        ("core", "core/src/lib.rs", vec![]),
        ("compiler_builtins", "compiler-builtins/compiler-builtins/src/lib.rs",
         vec!["--extern".to_string(), format!("core={}", core.display()), "--cfg".into(), "feature=\"compiler-builtins\"".into(),
              "--cfg".into(), "feature=\"unmangled-names\"".into(), "--cfg".into(), "feature=\"arch\"".into()]),
        ("alloc", "alloc/src/lib.rs", vec!["--extern".to_string(), format!("core={}", core.display()),
                                           "--extern".into(), format!("compiler_builtins={}", builtins.display())]),
    ] {
        println!("[web threads] building Rust's {name} with atomics");
        let status = Command::new(rustc())
            .env("RUSTC_BOOTSTRAP", "1") // only for these internal libraries; the app stays on stable
            .arg(src.join(file))
            .args(["--crate-name", name, "--edition=2024", "--crate-type=rlib", "--target", WASM, "-Cpanic=abort", "-Copt-level=2",
                   "-Ccodegen-units=1", "-Cdebuginfo=0", "-Ctarget-feature=+atomics,+bulk-memory,+simd128", "-Zforce-unstable-if-unmarked"])
            .arg("--sysroot").arg(dest)
            .arg("-L").arg(format!("dependency={}", libs.display()))
            .args(extra)
            .arg("-o").arg(libs.join(format!("lib{name}.rlib")))
            .status()
            .map_err(|e| format!("rustc: {e}"))?;
        if !status.success() {
            return Err(format!("building Rust's {name} with atomics failed"));
        }
    }
    fs::write(dest.join(".stamp"), stamp).map_err(|e| e.to_string())?;
    Ok(dest.to_path_buf())
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let o = crate::opts(args)?;
    let p = cargo::project(o.package.as_deref())?;
    let Some(lib) = p.package.lib.clone() else {
        return Err(format!("{}: the browser loads an app as a library: its code in src/lib.rs, src/main.rs only links it \
                            (the template of cargo jflib new)", p.package.name));
    };
    let explicit = o.modes.is_some();
    // a program with std: std for wasm32 comes prebuilt without atomics, so no threads mode
    let all = if p.package.std { "scalar,simd" } else { "scalar,simd,threads" };
    let mut modes: Vec<String> = o.modes.as_deref().unwrap_or(all).split(',').map(String::from).collect();
    if p.package.std && modes.iter().any(|m| m == "threads") {
        return Err("the browser's threads mode needs a standard library built with atomics, which rustup does not ship: \
                    a program with std (jflib's feature std) runs in the modes scalar and simd".to_string());
    }
    let work = p.target_dir.join("jflib-web");
    let mut sys = None;
    if modes.iter().any(|m| m == "threads") {
        match sysroot(&work.join("sysroot")) {
            Ok(s) => sys = Some(s),
            Err(e) if !explicit => {
                println!("note: {e}; built without the threads mode");
                modes.retain(|m| m != "threads");
            }
            Err(e) => return Err(e),
        }
    }
    let out = work.join(&p.package.name);
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut variants = Vec::new();
    for mode in &modes {
        let mut flags: Vec<String> = Vec::new();
        match mode.as_str() {
            "scalar" => {}
            "simd" => flags.extend(["-C".into(), "target-feature=+simd128".into()]),
            "threads" => {
                let s = sys.as_ref().map(|s| s.display().to_string()).unwrap_or_default();
                flags.extend(["-C", "target-feature=+simd128,+atomics,+bulk-memory", "--cfg", "web_threads", "-A", "unstable-features", "--sysroot"]
                    .map(String::from));
                flags.push(s);
            }
            other => return Err(format!("unknown mode {other} (scalar, simd, threads)")),
        }
        let mut cmd = cargo::command();
        cmd.args(["rustc", "-p", &p.package.name, "--lib", "--crate-type", "cdylib", "--target", WASM])
            .env("CARGO_TARGET_DIR", work.join(format!("cargo-{mode}")))
            .env("CARGO_ENCODED_RUSTFLAGS", flags.join("\x1f"))
            .env_remove("RUSTFLAGS");
        if o.release {
            cmd.arg("--release");
        }
        let wasm = cargo::build(&mut cmd)?
            .into_iter()
            .filter(|a| a.package_id == p.package.id)
            .flat_map(|a| a.filenames)
            .find(|f| f.extension().is_some_and(|x| x == "wasm"))
            .ok_or_else(|| format!("cargo built no wasm module of {lib}"))?;
        let file = format!("{}{}.wasm", p.package.name, if mode == "scalar" { String::new() } else { format!(".{mode}") });
        fs::copy(&wasm, out.join(&file)).map_err(|e| format!("{}: {e}", wasm.display()))?;
        variants.push(format!("\"{mode}\": \"{file}\""));
        println!("[web] {mode}: {}", out.join(&file).display());
    }
    for (name, data) in HOST {
        fs::write(out.join(name), data).map_err(|e| e.to_string())?;
    }
    fs::write(out.join("index.html"), INDEX.replace("{{APP}}", &p.package.name)).map_err(|e| e.to_string())?;
    fs::write(out.join("variants.json"), format!("{{{}}}", variants.join(", "))).map_err(|e| e.to_string())?;
    let n = crate::notices::copy(&p, WASM, &out.join("licenses"))?;
    println!("wrote {} (the modules, the host, the licences of {n} packages)", out.display());
    if let Some(port) = o.serve {
        serve(&out, port)?;
    }
    Ok(())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|x| x.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn answer(mut s: TcpStream, root: &Path) -> std::io::Result<()> {
    let mut first = String::new();
    let mut r = BufReader::new(s.try_clone()?);
    r.read_line(&mut first)?;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let path = first.split_whitespace().nth(1).unwrap_or("/").split(['?', '#']).next().unwrap_or("/").to_string();
    let rel = path.trim_start_matches('/');
    let file = root.join(if rel.is_empty() { "index.html" } else { rel });
    let inside = !rel.split('/').any(|c| c == "..");
    let (status, body, kind) = match fs::read(&file) {
        Ok(b) if inside => ("200 OK", b, content_type(&file)),
        _ => ("404 Not Found", b"not found".to_vec(), "text/plain"),
    };
    write!(s, "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCross-Origin-Opener-Policy: same-origin\r\n\
               Cross-Origin-Embedder-Policy: require-corp\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len())?;
    s.write_all(&body)
}

/// the bundle on 127.0.0.1:port with the headers the threads mode needs (cross-origin isolated)
fn serve(root: &Path, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    println!("http://127.0.0.1:{port}/  (Ctrl+C ends it)");
    for stream in listener.incoming().flatten() {
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let _ = answer(stream, &root);
        });
    }
    Ok(())
}
