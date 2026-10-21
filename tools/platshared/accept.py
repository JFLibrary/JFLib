#!/usr/bin/env python3
"""accept.py -- the whole acceptance of the plat-shared agent (R4): print, portal, plugin loader,
crash guard and monitor profile of the Rust rebuild against the assembly reference.

  python tools/platshared/accept.py [--ref build/rwref] [--rs build] [--fuzz N]

Runs (each also on its own):
  tools/platshared/print_rs_check.py        plat::print (layout, PDFs, the PDF printer)
  tools/platshared/clip_rs_check.py         clipboard DIB / BMP conversions (template's clip dib / file)
  tools/platshared/lib_check.py             guard, loader with the assembly's plugins, loader fuzz
  tools/platshared/portal_bus.py (in WSL)   OpenURI / OpenDirectory, FileChooser, Print on a fake bus
  icc (here)                                   plattest --test icc against codectest op 325 of the
                                               assembly (primary monitor: length, key, header)
  tools/platshared/icc_x11.py (in WSL)      the root window's _ICC_PROFILE on X11 (plattest
                                               --test iccx11, an unmapped window of its own)
Needs in --ref (a copy of the reference build, see tools/platshared/README.md): win/ and
linux/ PaintCraft, gfxtest, codectest, template; plugins/win/*.dll, plugins/linux/*.so (and the
Rust plattest + template built).  Exit code 0 = passed.
"""
import argparse, re, struct, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
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
    ap.add_argument("--fuzz", default="1200")
    a = ap.parse_args()
    py = sys.executable
    res = {}
    res["print"] = subprocess.run([py, str(HERE / "print_rs_check.py"), "--ref", a.ref, "--rs", a.rs], cwd=ROOT).returncode
    res["clip"] = subprocess.run([py, str(HERE / "clip_rs_check.py"), "--ref", a.ref, "--rs", a.rs], cwd=ROOT).returncode
    res["lib"] = subprocess.run([py, str(HERE / "lib_check.py"), "--ref", a.ref, "--rs", a.rs, "--fuzz", a.fuzz], cwd=ROOT).returncode
    res["portal"] = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e", "python3",
                                    "tools/platshared/portal_bus.py", "--ref", a.ref, "--rs", a.rs], cwd=ROOT).returncode
    res["icc"] = 0 if icc(a.ref, a.rs) else 1
    res["icc_x11"] = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e", "python3",
                                     "tools/platshared/icc_x11.py", "--rs", a.rs], cwd=ROOT).returncode
    print("accept:", ", ".join(f"{k} {'ok' if v == 0 else 'FAILED'}" for k, v in res.items()))
    return 1 if any(res.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
