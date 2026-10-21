"""Bounded, isolated unsafe/allocator review probes. Uses build_rs.py's compiler driver.

Native probes link the actual rt/simd crates. The ASan plugin probe changes only the
snapshot's `mod mem;` declaration: libc/ASan provide compiler memory entry points.
The real plugin ABI and effect wrappers are otherwise unchanged. No product source
is edited. RUSTC_BOOTSTRAP is set only for compiler subprocesses of the ASan probe.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build_rs as build

def write_json(path, data):
    path.write_bytes((json.dumps(data, indent=2) + "\n").encode())

def linux_path(path):
    path = Path(path).resolve()
    return "/mnt/" + path.drive[0].lower() + path.as_posix()[2:]

def run_log(out, name, cmd, env=None, timeout=60):
    try:
        p = subprocess.run([str(x) for x in cmd], cwd=ROOT, env=env, capture_output=True,
                           text=True, errors="replace", timeout=timeout)
        result = {"name": name, "exit": p.returncode, "timeout": False}
        text = p.stdout + p.stderr
    except subprocess.TimeoutExpired as e:
        result = {"name": name, "exit": None, "timeout": True}
        text = str(e.stdout or "") + str(e.stderr or "")
    (out / "logs" / (name + ".log")).write_bytes(text.encode())
    print(json.dumps(result), flush=True)
    return result

def compile_log(out, name, cmd, asan=False):
    original = build.run_cmd
    old_bootstrap = os.environ.get("RUSTC_BOOTSTRAP")
    if asan:
        os.environ["RUSTC_BOOTSTRAP"] = "1"
    try:
        # Build commands use the repository's rustc driver; diagnostics retained verbatim.
        old_stdout, old_stderr = sys.stdout, sys.stderr
        with (out / "logs" / (name + ".log")).open("w", encoding="utf-8") as log:
            sys.stdout = sys.stderr = log
            try:
                original(cmd)
            except build.BuildError as e:
                log.write(str(e))
                raise
            finally:
                sys.stdout, sys.stderr = old_stdout, old_stderr
    finally:
        if old_bootstrap is None:
            os.environ.pop("RUSTC_BOOTSTRAP", None)
        else:
            os.environ["RUSTC_BOOTSTRAP"] = old_bootstrap

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("mode", choices=["native", "asan", "tsan", "loader"])
    ap.add_argument("--out", default="build/PaintCraft-memory")
    args = ap.parse_args()
    os.chdir(ROOT)
    out = (ROOT / args.out).resolve()
    out.relative_to(ROOT / "build")
    (out / "logs").mkdir(parents=True, exist_ok=True)
    manifest = {}
    for directory in ["lib/rt", "lib/simd", "lib/plat", "lib/plugin", "modules/modrt", "modules/modplat"]:
        for src in (ROOT / directory).rglob("*.rs"):
            rel = src.relative_to(ROOT)
            manifest[rel.as_posix()] = hashlib.sha256(src.read_bytes()).hexdigest()
            dst = out / "baseline" / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            if not dst.exists():
                dst.write_bytes(src.read_bytes())
    manifest_path = out / (args.mode + "-source-manifest.json")
    write_json(manifest_path, manifest)
    results = []
    support = ROOT / "tools/memory_safety"
    if args.mode == "native":
        for target in ["win", "linux"]:
            builder = build.Builder(target, out / "native", False, 2)
            builder.build_libs(["rt", "simd"])
            exe = builder.dir / ("memory_probe.exe" if target == "win" else "memory_probe")
            cmd = ["rustc"] + builder.flags() + ["--crate-name", "memory_probe", "--crate-type", "bin",
                "-C", "debuginfo=2", "--extern", f"rt={builder.rlib('rt')}",
                "--extern", f"simd={builder.rlib('simd')}"]
            if target == "win":
                cmd += ["-C", f"linker={build.tool('lld-link')}", "-C", "link-arg=/ENTRY:mainCRTStartup",
                        "-C", "link-arg=/SUBSYSTEM:CONSOLE", "-C", "link-arg=/NODEFAULTLIB"]
            else:
                cmd += ["-C", f"linker={build.tool('ld.lld')}", "-C", "linker-flavor=ld",
                        "-C", "relocation-model=static", "-C", "target-feature=+crt-static",
                        "-C", "link-arg=-static", "-C", "link-arg=-nostdlib", "-C", "link-arg=-e_start"]
            compile_log(out, target + "-build", cmd + [support / "native_probe.rs", "-o", exe])
            launch = [exe] if target == "win" else ["wsl.exe", "-d", "Ubuntu-24.04", "--", "timeout", "30s", linux_path(exe)]
            results.append(run_log(out, target + "-native", launch))
            trait_cmd = ["rustc"] + builder.flags() + ["--crate-name", "guard_traits", "--crate-type", "rlib",
                "--emit=metadata", "--extern", f"rt={builder.rlib('rt')}", support / "guard_traits.rs",
                "-o", builder.dir / "guard_traits.rmeta"]
            results.append(run_log(out, target + "-guard-sync-witness", trait_cmd))
    elif args.mode == "asan":
        directory = out / "asan"
        directory.mkdir(exist_ok=True)
        snapshot = directory / "plugin"
        shutil.copytree(ROOT / "lib/plugin", snapshot, dirs_exist_ok=True)
        root = snapshot / "lib.rs"
        text = root.read_text(encoding="utf-8")
        assert text.count("mod mem;") == 1
        root.write_bytes(text.replace("mod mem;", "// Test host supplies libc/ASan memory intrinsics.").encode())
        common = ["--edition=2024", "--target=x86_64-unknown-linux-gnu", "-Cpanic=abort", "-Copt-level=1",
                  "-Cdebuginfo=2", "-Cforce-frame-pointers=yes", "-Zsanitizer=address", "-Zexternal-clangrt",
                  "-L", f"dependency={directory}"]
        plugin = directory / "libplugin.rlib"
        compile_log(out, "asan-plugin-build", ["rustc"] + common + ["--crate-type=rlib", "--crate-name=plugin", root, "-o", plugin], True)
        archive = directory / "libfxprobe.a"
        compile_log(out, "asan-fx-build", ["rustc"] + common + ["--crate-type=staticlib", "--crate-name=fxprobe",
                    "--extern", f"plugin={plugin}", support / "fx_probe.rs", "-o", archive], True)
        exe = directory / "fx_probe"
        results.append(run_log(out, "asan-link", ["wsl.exe", "-d", "Ubuntu-24.04", "--", "gcc",
            "-g", "-fno-omit-frame-pointer", "-fsanitize=address", "-no-pie", linux_path(support / "fx_host.c"),
            linux_path(archive), "-lpthread", "-ldl", "-lm", "-o", linux_path(exe)]))
        if results[-1]["exit"] == 0:
            for case in range(3):
                results.append(run_log(out, f"asan-fx-{case}", ["wsl.exe", "-d", "Ubuntu-24.04", "--", "timeout", "20s",
                    "env", "ASAN_OPTIONS=detect_leaks=1:halt_on_error=1:detect_stack_use_after_return=1", linux_path(exe), str(case)]))
    elif args.mode == "tsan":
        directory = out / "tsan"
        directory.mkdir(exist_ok=True)
        archive = directory / "libsyncprobe.a"
        common = ["--edition=2024", "--target=x86_64-unknown-linux-gnu", "-Cpanic=abort", "-Copt-level=1",
                  "-Cdebuginfo=2", "-Cforce-frame-pointers=yes", "-Zsanitizer=thread", "-Zexternal-clangrt"]
        compile_log(out, "tsan-build", ["rustc"] + common + ["--crate-type=staticlib", "--crate-name=syncprobe",
                    support / "sync_probe.rs", "-o", archive], True)
        exe = directory / "sync_probe"
        results.append(run_log(out, "tsan-link", ["wsl.exe", "-d", "Ubuntu-24.04", "--", "gcc",
            "-g", "-fno-omit-frame-pointer", "-fsanitize=thread", "-no-pie", linux_path(support / "sync_host.c"),
            linux_path(archive), "-lpthread", "-ldl", "-lm", "-o", linux_path(exe)]))
        if results[-1]["exit"] == 0:
            for case in range(2):
                results.append(run_log(out, f"tsan-sync-{case}", ["wsl.exe", "-d", "Ubuntu-24.04", "--", "timeout", "20s",
                    "env", "TSAN_OPTIONS=halt_on_error=1:exitcode=66", linux_path(exe), str(case)]))
    else:
        directory = out / "loader"
        directory.mkdir(exist_ok=True)
        builder = build.Builder("linux", out / "native", False, 2)
        builder.build_libs(["plat"])
        exe = directory / "loader_probe"
        cmd = ["rustc"] + builder.flags() + ["--crate-name", "loader_probe", "--crate-type", "bin", "-Cdebuginfo=2",
                "--extern", f"rt={builder.rlib('rt')}", "--extern", f"plat={builder.rlib('plat')}",
                "-C", f"linker={build.tool('ld.lld')}", "-C", "linker-flavor=ld", "-C", "relocation-model=static",
                "-C", "target-feature=+crt-static", "-C", "link-arg=-static", "-C", "link-arg=-nostdlib", "-C", "link-arg=-e_start",
                support / "loader_probe.rs", "-o", exe]
        compile_log(out, "loader-build", cmd)
        for case in ["valid", "symtab-gap", "strtab-gap"]:
            blob = bytearray(4097)
            blob[:16] = b"\x7fELF\x02\x01\x01" + b"\0" * 9
            struct.pack_into("<HHIQQQIHHHHHH", blob, 16, 3, 62, 1, 0, 64, 0, 0, 64, 56, 3, 0, 0, 0)
            for i, ph in enumerate([(1, 4, 0, 0, 0, 4096, 4096, 4096),
                                   (1, 4, 4096, 8192, 0, 1, 4096, 4096),
                                   (2, 4, 512, 512, 0, 96, 96, 8)]):
                struct.pack_into("<IIQQQQQQ", blob, 64 + 56 * i, *ph)
            symtab = 4096 if case == "symtab-gap" else 1280
            strtab = 4096 if case == "strtab-gap" else 768
            for i, entry in enumerate([(4, 1024), (5, strtab), (10, 32), (6, symtab), (11, 24), (0, 0)]):
                struct.pack_into("<QQ", blob, 512 + 16 * i, *entry)
            struct.pack_into("<II", blob, 1024, 1, 2)
            struct.pack_into("<IBBHQQ", blob, 1280 + 24, 1, 0x11, 0, 1, 2048, 1)
            blob[768:776] = b"\0review\0"
            path = directory / (case + ".so")
            path.write_bytes(blob)
            results.append(run_log(out, "loader-" + case, ["wsl.exe", "-d", "Ubuntu-24.04", "--", "timeout", "20s",
                linux_path(exe), linux_path(path)]))
    write_json(out / (args.mode + "-results.json"), results)

if __name__ == "__main__":
    main()
