#!/usr/bin/env python3
"""Golden renders of every app, for the style-layer refactor: the WinUI look must stay
byte-identical while the drawing code moves from ASMLIB/src/ui into ASMLIB/styles/winui.

Renders (each target):
  * every `shot F ...` of every apps/*/tests/*.uis script (run with its #!args), hashed right
    after the run -- covers dialogs, popups, tool windows, PaintCraft states;
  * each app's default frame via --shot, light / dark x 100 / 150 %;
  * the gallery page scrolled step by step (light / dark x 100 / 150 %).
The hashes (SHA-1 of the PNG bytes; our PNG encoder is deterministic) go to
<golden>/<target>.json.

  python tools/style_golden.py --record [--target both] [--out build] [--golden build/golden]
  python tools/style_golden.py --check  [...]      exit 1 and list every render that differs
Options: --no-build (use the binaries in --out), --apps a,b (subset), --style NAME (passed to
build.py once the style layer exists).
"""
import argparse, hashlib, json, os, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402

PAGES = 14                      # gallery scroll steps (600 DIP each)


def sha(p):
    return hashlib.sha1(p.read_bytes()).hexdigest()


def run(exe, target, args):
    cmd = build.run_cmd_for(exe, target, args)
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def shots_of(script):
    out = []
    for line in script.read_text(encoding="utf-8").splitlines():
        t = line.split()
        if len(t) >= 2 and t[0] == "shot":
            out.append(t[1])
    return out


def gallery_script(gdir, target, theme, scale):
    lines = ["size 1280 800", f"theme {theme}", f"scale {scale}", "wait 50"]
    names = []
    for i in range(PAGES):
        f = f"{gdir}/{target}_gallery_{theme}_{scale}_{i:02d}.png"
        lines.append(f"shot {f}")
        names.append(f)
        lines += ["move 640,400", "wheel 640,400 -600", "wait 50"]
    p = ROOT / gdir / f"{target}_gallery_{theme}_{scale}.uis"
    p.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    return p, names


def collect(target, out, gdir, apps, log):
    res = {}
    for app in apps:
        exe = build.exe_path(app, target, out)
        if not exe.exists():
            log(f"[{target}] skip {app}: no binary")
            continue
        # default frames
        for theme in ("light", "dark"):
            for scale in (100, 150):
                f = ROOT / gdir / f"{target}_{app}_{theme}_{scale}.png"
                if f.exists():
                    f.unlink()
                rc, _ = run(exe, target, ["--shot", str(f.relative_to(ROOT)).replace("\\", "/"),
                                          "--theme", theme, "--scale", str(scale), "--anim", "off"])
                if f.exists():                  # (apps without a UI write no frame)
                    res[f"{app}/default_{theme}_{scale}"] = sha(f)
        # gallery pages
        if app == "gallery":
            for theme in ("light", "dark"):
                for scale in (100, 150):
                    sp, names = gallery_script(gdir, target, theme, scale)
                    run(exe, target, ["--anim", "off", "--script", str(sp.relative_to(ROOT)).replace("\\", "/")])
                    for n in names:
                        p = ROOT / n
                        res[f"gallery/page_{Path(n).stem.split('_gallery_')[1]}"] = sha(p) if p.exists() else "missing"
        # shots of the test scripts
        tdir = build.APPS / app / "tests"
        for script in sorted(tdir.glob("*.uis")) if tdir.is_dir() else []:
            shots = shots_of(script)
            if not shots:
                continue
            for s in shots:
                p = ROOT / s
                p.parent.mkdir(parents=True, exist_ok=True)
                if p.exists():
                    p.unlink()
            args = ["--script", str(script.relative_to(ROOT)).replace("\\", "/")]
            first = script.read_text(encoding="utf-8").splitlines()[0]
            if first.startswith("#!args "):
                args = first[7:].split() + args
            rc, _ = run(exe, target, args)
            for k, s in enumerate(shots):
                p = ROOT / s
                res[f"{app}/{script.name}#{k}:{s}"] = sha(p) if p.exists() else f"missing rc={rc}"
        log(f"[{target}] {app}: {sum(1 for k in res if k.startswith(app + '/'))} renders")
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--record", action="store_true")
    g.add_argument("--check", action="store_true")
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="legacy/build")
    ap.add_argument("--golden", default="build/golden")
    ap.add_argument("--apps", default="")
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    build.OUT_DIR = out
    gdir = Path(a.golden)
    (ROOT / gdir).mkdir(parents=True, exist_ok=True)
    apps = [x for x in a.apps.split(",") if x] or build.app_names()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    bad = 0
    for t in targets:
        tapps = [x for x in apps if t in build.app_targets(x)]      # apps/NAME/TARGETS
        if not a.no_build:
            for app in tapps:
                build.build_app(app, t, out)
        res = collect(t, out, gdir.as_posix(), tapps, print)
        jf = ROOT / gdir / f"{t}.json"
        if a.record:
            old = json.loads(jf.read_text()) if jf.exists() and a.apps else {}
            old.update(res)
            jf.write_text(json.dumps(old, indent=0, sort_keys=True))
            miss = [k for k, v in res.items() if v.startswith("missing")]
            print(f"[{t}] recorded {len(res)} renders" + (f", {len(miss)} missing: {miss[:5]}" if miss else ""))
        else:
            ref = json.loads(jf.read_text())
            diff = [k for k, v in res.items() if ref.get(k) != v]
            bad += len(diff)
            for k in diff[:40]:
                print(f"[{t}] DIFF {k}: {ref.get(k)} -> {res[k]}")
            print(f"[{t}] {len(res) - len(diff)}/{len(res)} renders identical")
    if a.check:
        print("style_golden:", "ALL IDENTICAL" if bad == 0 else f"{bad} differ")
        return 1 if bad else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
