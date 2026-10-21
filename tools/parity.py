#!/usr/bin/env python3
"""parity.py -- the acceptance test of the rebuild: assembly reference against Rust.

  python tools/parity.py app NAME [--target win|linux|both] [--no-scripts] [--no-matrix] [--style S]
  python tools/parity.py shot NAME [--target T] -- APP ARGS     one --shot run, compared

For an app it runs
  * the shot matrix: --theme light|dark x --scale 100|150, --anim off, with --dump
    (pixels and widget dumps compared), and
  * every apps/NAME/tests/*.uis (`#!args` honoured): exit code, stdout, stderr and every
    file the script writes (`shot` -> pixels, `dump` -> text) compared.
The reference is build/rwref/<target>/NAME (built with legacy/build.py when missing, or with
--build-ref), the rebuild build/<target>/NAME (build_rs.py).  Pixels are compared,
not PNG bytes.  Exit code 0 = no difference.

apps/NAME/parity.txt (optional) adjusts an app, one setting per line ('#' comments):
  matrix off              no shot / dump matrix (apps without a UI)
  stdout checks           compare only stdout lines that do not start with two blanks
                          (self tests: their detail lines hold times and paths)
  pending FILE.uis WHY    a script the rebuild does not support yet (skipped, reported)
  rsonly FILE.uis WHY     a script of the Rust build only (no assembly counterpart: skipped here,
                          run by build_rs.py test)
  mask FILE.uis SHOT X Y W H WHY   pixels of that shot inside the rectangle are not compared
                          (an intended difference, e.g. a better encoder's file size)
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT))
from pngutil import read_png  # noqa: E402
from build_rs import app_targets, plugin_test_dir, run_exe, script_args  # noqa: E402  (r12-inst: app_targets)

REF = Path(os.environ.get("jfgui_REF") or ROOT / "build" / "rwref")  # jfgui_REF: a shared reference build
RS = ROOT / "build"
TMP = ROOT / "build" / "parity"


def app_config(name):
    """apps/NAME/parity.txt -> {"matrix": bool, "stdout": "all"|"checks", "pending": {file: why}}"""
    cfg = {"matrix": True, "stdout": "all", "pending": {}, "rsonly": {}, "masks": {}}
    f = ROOT / "apps" / name / "parity.txt"
    if f.exists():
        for ln in f.read_text(encoding="utf-8").splitlines():
            p = ln.split("#")[0].split(None, 2)
            if p[:2] == ["matrix", "off"]:
                cfg["matrix"] = False
            elif p[:2] == ["stdout", "checks"]:
                cfg["stdout"] = "checks"
            elif len(p) >= 2 and p[0] == "pending":
                cfg["pending"][p[1]] = p[2].strip() if len(p) > 2 else ""
            elif len(p) >= 2 and p[0] == "rsonly":
                cfg["rsonly"][p[1]] = p[2].strip() if len(p) > 2 else ""
            elif len(p) >= 3 and p[0] == "mask":
                # mask FILE.uis SHOT.png X Y W H WHY: an intended difference inside a rectangle
                q = ln.split("#")[0].split(None, 7)
                cfg["masks"].setdefault((q[1], q[2]), []).append(tuple(int(v) for v in q[3:7]))
    return cfg


def stdout_of(text, mode):
    if mode == "checks":
        return [ln for ln in text.splitlines() if not ln.startswith("  ")]
    return text


def exe(base, target, name):
    return base / target / (name + (".exe" if target == "win" else ""))


def ensure_ref(name, targets, force, style=None):
    for t in targets:
        if style and not exe(REF, t, name).exists():
            raise SystemExit(f"no asm reference {exe(REF, t, name)} for style {style}: build it with\n"
                             f"  python legacy/build.py app {name} --style {style} --target {t} --out {REF}")
        if style:
            continue
        if force or not exe(REF, t, name).exists():
            subprocess.run([sys.executable, str(ROOT.parent / "legacy" / "build.py"), "app", name, "--target", t, "--out", str(REF)],
                           cwd=ROOT, check=True, capture_output=True)


def ensure_rs(name, targets, style=None):
    for t in targets:
        sty = ["--style", style, "--out", str(RS)] if style else []
        r = subprocess.run([sys.executable, "build_rs.py", "app", name, "--target", t] + sty,
                           cwd=ROOT, capture_output=True, text=True)
        if r.returncode:
            sys.stdout.write(r.stdout + r.stderr)
            raise SystemExit("rs build failed")


def cmp_pixels(a, b, masks=()):
    try:
        wa, ha, pa = read_png(a)
        wb, hb, pb = read_png(b)
    except Exception as e:  # noqa: BLE001
        return f"unreadable ({e})"
    if (wa, ha) != (wb, hb):
        return f"size {wa}x{ha} vs {wb}x{hb}"
    n = 0
    first = None
    for i in range(0, len(pa), 4):
        if pa[i:i + 3] != pb[i:i + 3]:
            x, y = (i // 4) % wa, (i // 4) // wa
            if any(mx <= x < mx + mw and my <= y < my + mh for mx, my, mw, mh in masks):
                continue
            n += 1
            if first is None:
                first = ((i // 4) % wa, (i // 4) // wa)
    return None if n == 0 else f"{n} pixels differ, first at {first}"


def outputs_of(uis):
    files = []
    for ln in uis.read_text(encoding="utf-8").splitlines():
        p = ln.split()
        if len(p) > 1 and p[0] in ("shot", "dump"):
            files.append((p[0], ROOT / p[1]))
    return files


def run_script(base, target, name, uis, keep):
    if name == "PaintCraft" and uis.name.startswith("plugins"):
        plugin_test_dir(base, target)  # the plugins scripts' `--plugins @ptest` (as build.py test)
    for _, f in outputs_of(uis):
        f.parent.mkdir(parents=True, exist_ok=True)
        if f.exists():
            f.unlink()
    r = run_exe(target, exe(base, target, name), script_args(uis) + ["--script", uis.relative_to(ROOT).as_posix()])
    saved = []
    for kind, f in outputs_of(uis):
        dst = keep / f.name
        if f.exists():
            shutil.copy(f, dst)
            saved.append((kind, dst))
        else:
            saved.append((kind, None))
    return r, saved


ADDR_IDS = (0xB0000000, 0xCFFFFFFF)   # widget ids hashed from static addresses (menus, menu bar
                                     # items): they differ between the asm and Rust by nature
ADDR_ID_RE = re.compile(r"(?<=[:\[,])(\d{10})(?=[,\]}])")


def mask_ids(text):
    """dump text with the address-hashed ids replaced by a marker"""
    return ADDR_ID_RE.sub(lambda m: '"addr"' if ADDR_IDS[0] <= int(m.group(1)) <= ADDR_IDS[1] else m.group(1), text)


def parity_app(name, targets, scripts, matrix):
    cfg = app_config(name)
    matrix = matrix and cfg["matrix"]
    diffs = 0
    checks = 0
    pending = 0
    for t in targets:
        if matrix:
            for theme in ("light", "dark"):
                for scale in ("100", "150"):
                    tag = f"{theme}{scale}"
                    d = TMP / name / t / "matrix"
                    d.mkdir(parents=True, exist_ok=True)
                    res = {}
                    for side, base in (("ref", REF), ("rs", RS)):
                        png = d / f"{side}_{tag}.png"
                        r = run_exe(t, exe(base, t, name), ["--anim", "off", "--theme", theme, "--scale", scale,
                                                            "--shot", png.relative_to(ROOT).as_posix(), "--dump"])
                        res[side] = (png, [mask_ids(x) for x in r.stdout.splitlines() if x.startswith("{")], r.returncode)
                    checks += 1
                    why = cmp_pixels(res["ref"][0], res["rs"][0])
                    if res["ref"][1] != res["rs"][1]:
                        why = (why + "; " if why else "") + "dump differs"
                    if res["ref"][2] != res["rs"][2]:
                        why = (why + "; " if why else "") + f"exit {res['ref'][2]} vs {res['rs'][2]}"
                    if why:
                        diffs += 1
                        print(f"DIFF [{t}] {name} shot {tag}: {why}")
        if scripts:
            for uis in sorted((ROOT / "apps" / name / "tests").glob("*.uis")):
                if uis.name in cfg["rsonly"]:
                    continue
                if uis.name in cfg["pending"]:
                    pending += 1
                    continue
                d = TMP / name / t / uis.stem
                for side in ("ref", "rs"):
                    (d / side).mkdir(parents=True, exist_ok=True)
                rr, fr = run_script(REF, t, name, uis, d / "ref")
                rs, fs = run_script(RS, t, name, uis, d / "rs")
                checks += 1
                why = []
                if rr.returncode != rs.returncode:
                    why.append(f"exit {rr.returncode} vs {rs.returncode}")
                if stdout_of(rr.stdout, cfg["stdout"]) != stdout_of(rs.stdout, cfg["stdout"]):
                    why.append("stdout differs")
                if rr.stderr != rs.stderr:
                    why.append("stderr differs")
                for (kind, a), (_, b) in zip(fr, fs):
                    if (a is None) != (b is None):
                        why.append(f"{kind} file missing on one side")
                    elif a is not None:
                        if kind == "shot":
                            w = cmp_pixels(a, b, cfg["masks"].get((uis.name, a.name), ()))
                            if w:
                                why.append(f"shot {a.name}: {w}")
                        elif mask_ids(a.read_bytes().decode("utf-8", "replace")) != mask_ids(b.read_bytes().decode("utf-8", "replace")):
                            why.append(f"dump {a.name} differs")
                if why:
                    diffs += 1
                    print(f"DIFF [{t}] {name}/{uis.name}: {'; '.join(why)}")
    extra = f", {pending} pending ({', '.join(sorted(cfg['pending']))})" if pending else ""
    print(f"parity {name}: {checks - diffs} of {checks} identical{extra}")
    return diffs == 0


def main():
    argv = sys.argv[1:]
    extra = []
    if "--" in argv:
        i = argv.index("--")
        argv, extra = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["app", "shot"])
    ap.add_argument("name")
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--no-scripts", action="store_true")
    ap.add_argument("--no-matrix", action="store_true")
    ap.add_argument("--build-ref", action="store_true")
    ap.add_argument("--style", help="another style: asm reference build/rwref_S (jfgui_REF + _S), "
                                    "Rust build build/rs_S")
    a = ap.parse_args(argv)
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    targets = [t for t in targets if t in app_targets(a.name)]  # r12-inst: apps/NAME/TARGETS (installer: win)
    if a.style:
        global REF, RS, TMP
        REF = Path(f"{REF}_{a.style}")
        RS = ROOT / "build" / f"rs_{a.style}"
        TMP = ROOT / "build" / f"parity_{a.style}"
    ensure_ref(a.name, targets, a.build_ref, a.style)
    ensure_rs(a.name, targets, a.style)
    if a.cmd == "app":
        ok = parity_app(a.name, targets, not a.no_scripts, not a.no_matrix)
    else:
        ok = True
        for t in targets:
            d = TMP / a.name / t / "shot"
            d.mkdir(parents=True, exist_ok=True)
            pngs = []
            for side, base in (("ref", REF), ("rs", RS)):
                png = d / f"{side}.png"
                run_exe(t, exe(base, t, a.name), extra + ["--shot", png.relative_to(ROOT).as_posix()])
                pngs.append(png)
            w = cmp_pixels(*pngs)
            print(f"[{t}] {a.name} {' '.join(extra)}: {w or 'identical'}")
            ok &= w is None
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
