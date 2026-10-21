"""Build Rust's own core/alloc/builtins for shared WASM memory, with direct rustc.

No Cargo or third-party dependencies. rust-src must match the installed compiler.
RUSTC_BOOTSTRAP is confined to compiling these internal Rust libraries; application
crates continue to use the ordinary stable compiler and warnings policy.
"""
import hashlib
import os
from pathlib import Path
import subprocess


def ensure(destination):
    destination = Path(destination).resolve()
    rustc = subprocess.check_output(["rustc", "--version", "--verbose"], text=True)
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    source = sysroot / "lib/rustlib/src/rust/library"
    if not (source / "core/src/lib.rs").exists():
        raise RuntimeError("Shared WASM requires matching Rust sources: rustup component add rust-src")
    fingerprint = hashlib.sha256((rustc + Path(__file__).read_text()).encode()).hexdigest()
    stamp = destination / ".stamp"
    libraries = destination / "lib/rustlib/wasm32-unknown-unknown/lib"
    if stamp.exists() and stamp.read_text() == fingerprint and all((libraries / f"lib{n}.rlib").exists() for n in ("core", "alloc", "compiler_builtins")):
        return destination
    libraries.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, RUSTC_BOOTSTRAP="1")
    flags = ["--edition=2024", "--crate-type=rlib", "--target=wasm32-unknown-unknown",
             "-Cpanic=abort", "-Copt-level=2", "-Ccodegen-units=1", "-Cdebuginfo=0",
             "-Ctarget-feature=+atomics,+bulk-memory,+simd128", "-Zforce-unstable-if-unmarked",
             "--sysroot", str(destination), "-L", f"dependency={libraries}"]
    for name, file, extra in [
        ("core", "core/src/lib.rs", []),
        ("compiler_builtins", "compiler-builtins/compiler-builtins/src/lib.rs",
         ["--extern", f"core={libraries / 'libcore.rlib'}", "--cfg", 'feature="compiler-builtins"',
          "--cfg", 'feature="unmangled-names"', "--cfg", 'feature="arch"']),
        ("alloc", "alloc/src/lib.rs", ["--extern", f"core={libraries / 'libcore.rlib'}",
                                       "--extern", f"compiler_builtins={libraries / 'libcompiler_builtins.rlib'}"]),
    ]:
        print(f"[web threads] building Rust {name} with atomics", flush=True)
        subprocess.run(["rustc", str(source / file), "--crate-name", name, *flags, *extra,
                        "-o", str(libraries / f"lib{name}.rlib")], env=environment, check=True)
    stamp.write_text(fingerprint)
    return destination


if __name__ == "__main__":
    import sys
    ensure(sys.argv[1])
