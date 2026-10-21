"""states.py APP [UIS...] [--ref DIR] [--rs DIR] [--linux] [--noref] -- style-kde's extra
state scripts (default: the s*.uis next to this file; hover / pressed / focus states, menus,
windows, the CSD title bar, a custom accent) run on the asm reference (DIR/<target>/APP,
default $jfgui_REF_kde or build/rwref_kde) and the Rust build (default build/rs_kde); every
output is compared: exit code, stdout (address ids masked), stderr, shots (pixels).
--linux adds the Linux runs: asm linux against rs linux when the reference has a Linux build,
else rs win against rs linux.  A script line `#!args ...` adds program arguments,
`#!env K=V ...` environment variables.  The scripts write their shots to build/kdes/.

  python styles/kde/states/states.py gallery --linux
  python styles/kde/states/states.py gallery --ref build/oldref_kde      (old asm build, win)
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tools"))
import parity  # noqa: E402
from build_rs import wsl_path  # noqa: E402
from pngutil import read_png  # noqa: E402

args = sys.argv[1:]
ref = Path(os.environ["jfgui_REF"] + "_kde") if os.environ.get("jfgui_REF") else ROOT / "build" / "rwref_kde"
rs = ROOT / "build" / "rs_kde"
linux = noref = False
files = []
app = args.pop(0)
while args:
    a = args.pop(0)
    if a == "--ref":
        ref = Path(args.pop(0))
        if not ref.is_absolute():
            ref = ROOT / ref
    elif a == "--rs":
        rs = Path(args.pop(0))
        if not rs.is_absolute():
            rs = ROOT / rs
    elif a == "--linux":
        linux = True
    elif a == "--noref":
        noref = True
    else:
        files.append(Path(a))
if not files:
    files = sorted(Path(__file__).resolve().parent.glob("s*.uis"))


def header(uis, key):
    out = []
    for ln in uis.read_text(encoding="utf-8").splitlines():
        if ln.startswith(key):
            out += ln[len(key):].split()
    return out


def run(base, target, uis, keep):
    for _, f in parity.outputs_of(uis):
        f.parent.mkdir(parents=True, exist_ok=True)
        if f.exists():
            f.unlink()
    exe = parity.exe(base, target, app)
    a = header(uis, "#!args") + ["--script", uis.resolve().relative_to(ROOT).as_posix()]
    envs = header(uis, "#!env")
    if target == "win":
        env = dict(os.environ)
        for e in envs:
            k, v = e.split("=", 1)
            env[k] = v
        cmd = [str(exe)] + a
    else:
        env = None
        cmd = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", wsl_path(ROOT), "-e", "env"] + envs + [wsl_path(exe)] + a
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=ROOT,
                       timeout=900, env=env)
    if keep.exists():
        shutil.rmtree(keep)
    keep.mkdir(parents=True)
    saved = []
    for kind, f in parity.outputs_of(uis):
        dst = keep / f.name
        if f.exists():
            shutil.copy(f, dst)
            saved.append((kind, dst))
        else:
            saved.append((kind, None))
    return r, saved


def same_px(a, b):
    try:
        if read_png(a) == read_png(b):
            return None
    except Exception:  # noqa: BLE001
        pass
    return parity.cmp_pixels(a, b)


def compare(x, y):
    (ra, fa), (rb, fb) = x, y
    why = []
    if ra.returncode != rb.returncode:
        why.append(f"exit {ra.returncode} vs {rb.returncode}")
    if parity.mask_ids(ra.stdout) != parity.mask_ids(rb.stdout):
        why.append("stdout differs")
    if ra.stderr != rb.stderr:
        why.append("stderr differs")
    n = 0
    for (kind, a), (_, b) in zip(fa, fb):
        if (a is None) != (b is None):
            why.append(f"{kind} {a or b} missing on one side")
        elif a is not None:
            n += 1
            if kind == "shot":
                w = same_px(a, b)
                if w:
                    why.append(f"shot {a.name}: {w}")
    return why, n


total = bad = 0
for uis in files:
    out = ROOT / "build" / "kde_states" / uis.stem
    res = {}
    if not noref:
        res["ref"] = run(ref, "win", uis, out / "ref_win")
    res["rs"] = run(rs, "win", uis, out / "rs_win")
    ref_linux = linux and not noref and parity.exe(ref, "linux", app).exists()
    if linux:
        res["rsl"] = run(rs, "linux", uis, out / "rs_linux")
    if ref_linux:
        res["refl"] = run(ref, "linux", uis, out / "ref_linux")
    pairs = []
    if not noref:
        pairs.append(("asm win / rs win", "ref", "rs"))
    if ref_linux:
        pairs.append(("asm lx / rs lx", "refl", "rsl"))
    elif linux:
        pairs.append(("rs win / rs linux", "rs", "rsl"))
    for label, a, b in pairs:
        why, n = compare(res[a], res[b])
        total += 1
        bad += bool(why)
        print(f"{uis.name:16s} {label:18s} exit {res[a][0].returncode}/{res[b][0].returncode} {n:3d} files  "
              + ("same" if not why else "DIFF: " + "; ".join(why[:12])))
print(f"states {app}: {total - bad} of {total} identical")
