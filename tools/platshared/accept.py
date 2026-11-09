#!/usr/bin/env python3
"""accept.py -- what is left of the acceptance of the plat-shared agent (R4): the portal and the
monitor profile of the Rust rebuild against the assembly reference.  (Its print, clipboard and
plugin-loader parts imported PaintCraft's check scripts, which moved with the app to its own
repository, JFLibrary/JFPainter, in 2026-11.)

  python tools/platshared/accept.py [--ref build/rwref] [--rs build]

Runs (each also on its own):
  tools/platshared/portal_bus.py (in WSL)   OpenURI / OpenDirectory, FileChooser on a fake bus
  icc (here)                                   plattest --test icc against codectest op 325 of the
                                               assembly (primary monitor: length, key, header)
  tools/platshared/icc_x11.py (in WSL)      the root window's _ICC_PROFILE on X11 (plattest
                                               --test iccx11, an unmapped window of its own)
Needs in --ref (a copy of the reference build, see tools/platshared/README.md): win/ and
linux/ gfxtest and codectest (and the Rust plattest built).  Exit code 0 = passed.
"""
import argparse, re, struct, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))


def icc(ref, rs):
    import imgtest_common as it
    r = subprocess.run([str(ROOT / rs / "win" / "plattest.exe"), "--test", "icc"], cwd=ROOT, capture_output=True, text=True)
    m = re.search(r"icc: primary monitor: (\d+) bytes, fnv \w+, key (\w+)", r.stdout)
    h = re.search(r"icc: head (\w+)", r.stdout)
    runner = it.Runner("win", ref, work_name="platshared_icc")
    res, out = runner.one(325, 0xFFFFFFFF, b"")
    n, = struct.unpack_from("<I", out, 0)
    key, = struct.unpack_from("<Q", out, 4)
    ok = r.returncode == 0
    if res == 1:
        ok = ok and m and int(m.group(1)) == n and int(m.group(2), 16) == key and h and bytes.fromhex(h.group(1)) == out[12:12 + 128]
        what = f"profile of {n} bytes, key {key:#x}"
    else:
        ok = ok and "none" in r.stdout
        what = "no profile"
    print(("PASS" if ok else "FAIL") + f" [win] icc: the primary monitor's profile as the assembly's ({what})")
    if not ok:
        print(r.stdout[-600:])
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", default="build/rwref")
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    res = {}
    res["portal"] = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e", "python3",
                                    "tools/platshared/portal_bus.py", "--ref", a.ref, "--rs", a.rs], cwd=ROOT).returncode
    res["icc"] = 0 if icc(a.ref, a.rs) else 1
    res["icc_x11"] = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e", "python3",
                                     "tools/platshared/icc_x11.py", "--rs", a.rs], cwd=ROOT).returncode
    print("accept:", ", ".join(f"{k} {'ok' if v == 0 else 'FAILED'}" for k, v in res.items()))
    return 1 if any(res.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
