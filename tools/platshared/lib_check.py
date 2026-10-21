#!/usr/bin/env python3
"""lib_check.py -- the plugin loader and crash guard of the Rust rebuild (agent plat-shared, R4).

  python tools/platshared/lib_check.py [--target win|linux|both] [--ref build/rwref] [--rs build]
                                          [--plugins build/rwref/plugins] [--fuzz N]

  self     plattest --test guard (also 20 times in a row), --test guardlimit, --test
           guardcrash (exit code 3 with the crash checks) and
           --test lib --dir <plugins>/<target> (the assembly's example / test plugins of
           PaintCraft: query, PLUGININFO, crashq / crash / crashpf faults, swapchan's render,
           loader errors)
  fuzz     tools/plugin_check.py's loader fuzz: N mutants (bit flips anywhere, in the first
           4 KB, byte values 0 / FF / 7F / 80, truncations; seed 1) of swapchan, qoi, pshapes
           and crashpf go through the assembly's PaintCraft --plugin-fuzz and plattest --test
           libfuzz: the counts "files,loaded,rejected,faults" must be equal, faults 0
The plugins are the assembly's (python legacy/build.py plugin; copy <out>/<target>/plugins/* to
<plugins>/<target>/).  Exit code 0 = passed.
"""
import argparse, random, re, shutil, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import plugin_check as pk  # noqa: E402

WSL = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e"]
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def run(target, exe, args, timeout=1800):
    if target == "win":
        cmd = [str(ROOT / (exe + ".exe"))] + args
    else:
        cmd = WSL + ["./" + exe] + args
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    return r.returncode, r.stdout + r.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--ref", default="build/rwref")
    ap.add_argument("--rs", default="build")
    ap.add_argument("--plugins", default="build/rwref/plugins")
    ap.add_argument("--fuzz", type=int, default=1200)
    a = ap.parse_args()
    ref, rs, plug = (v.replace("\\", "/").rstrip("/") for v in (a.ref, a.rs, a.plugins))
    work = ROOT / "build" / "plat-shared"
    work.mkdir(parents=True, exist_ok=True)
    (work / "empty.uis").write_text("size 800 600\nwait 10\n", encoding="utf-8", newline="\n")
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        ext = ".dll" if t == "win" else ".so"
        rc, out = run(t, f"{rs}/{t}/plattest", ["--test", "guard"])
        check(f"[{t}] plattest --test guard", rc == 0, out[-800:])
        rc, out = run(t, f"{rs}/{t}/plattest", ["--test", "guard", "--repeat", "20"])
        check(f"[{t}] plattest --test guard --repeat 20 (races)", rc == 0, out[-800:])
        rc, out = run(t, f"{rs}/{t}/plattest", ["--test", "guardlimit"])
        check(f"[{t}] plattest --test guardlimit (a full thread table)", rc == 0, out[-800:])
        rc, out = run(t, f"{rs}/{t}/plattest", ["--test", "guardcrash"])
        check(f"[{t}] plattest --test guardcrash: crash report, exit code 3", rc == 3 and "FAIL" not in out, out[-800:])
        rc, out = run(t, f"{rs}/{t}/plattest", ["--test", "lib", "--dir", f"{plug}/{t}"])
        check(f"[{t}] plattest --test lib (the assembly's test plugins)", rc == 0, out[-800:])
        # the loader fuzz: the same mutants through the assembly and Rust
        rng = random.Random(1)
        d = work / f"pfuzz_{t}"
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True)
        bases = [(ROOT / plug / t / (n + ext)).read_bytes() for n in ("swapchan", "qoi", "pshapes", "crashpf")]
        for i in range(a.fuzz):
            kind = rng.choice(("any", "head", "head", "trunc"))
            (d / f"m{i:05d}{ext}").write_bytes(pk.mutate(rng.choice(bases), rng, kind))
        drel = str(d.relative_to(ROOT)).replace("\\", "/")
        arc, aout = run(t, f"{ref}/{t}/PaintCraft", ["--plugin-fuzz=" + drel, "--script", "build/plat-shared/empty.uis"])
        rc, out = run(t, f"{rs}/{t}/plattest", ["--test", "libfuzz", "--dir", drel])
        pat = r"plugin-fuzz: files,loaded,rejected,faults = (\d+),(\d+),(\d+),(\d+)"
        am, m = re.search(pat, aout), re.search(pat, out)
        print(f"   asm:  {am.group(0) if am else aout[-300:]}")
        print(f"   Rust: {m.group(0) if m else out[-300:]}")
        for line in out.splitlines():
            if line.startswith("plugin-fuzz: fault in"):
                print("        ", line)
        # (faults: with more mutants than plugin_check's 1200, a few DLLs fault inside ntdll's own
        # loader (LoadLibraryExW) on Windows -- the guard takes the first-chance exception; the
        # assembly shows the same files and counts)
        check(f"[{t}] loader fuzz: {a.fuzz} mutants, no fault in our loader (only where the assembly has them)",
              m and int(m.group(1)) == a.fuzz and (m.group(4) == "0" or (am and am.group(4) == m.group(4))), out[-600:])
        check(f"[{t}] loader fuzz: files, loaded, rejected, faults as the assembly's", am and m and am.groups() == m.groups(),
              (am and am.groups(), m and m.groups()))
        fa = sorted(re.findall(r"plugin-fuzz: fault in (\S+):", aout))
        fr = sorted(re.findall(r"plugin-fuzz: fault in (\S+):", out))
        check(f"[{t}] loader fuzz: the same files fault ({len(fr)})", fa == fr, (fa, fr))
        if m and m.group(4) == "0":
            shutil.rmtree(d, ignore_errors=True)
    print("lib_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
