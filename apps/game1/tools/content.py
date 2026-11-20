#!/usr/bin/env python3
"""content.py -- rebuild game 1's content: every apps/game1/content/*.jfs through modeltool (JFScript -> lib/geom ->
model.glb, its pictures and report) and game1 --bake (model.glb -> NAME.g1m, the file the client and the server read).

  python apps/game1/tools/content.py            build modeltool and game1 if they are missing, rebuild every .g1m
  python apps/game1/tools/content.py --check    rebuild into build/game1-content/check/ and compare byte for byte
                                                with the checked-in files (exit code 1 when one differs)
  python apps/game1/tools/content.py --build    build modeltool and game1 first, also when they exist

The pictures of each model land in build/game1-content/NAME/ (sheet.png: front, side, top, three-quarter).  The
content is deterministic: modeltool runs JFScript with software floats, the bake reads the glTF's f32 as they are
(docs/GAME1.md, "Karte")."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CONTENT = ROOT / "apps" / "game1" / "content"
OUT = ROOT / "build" / "game1-content"
EXE = ".exe" if sys.platform == "win32" else ""
TARGET = "win" if sys.platform == "win32" else "linux"


def program(name):
    return ROOT / "build" / TARGET / (name + EXE)


def run(args):
    r = subprocess.run([str(a) for a in args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
    sys.stdout.write(r.stdout)
    if r.returncode != 0:
        sys.stdout.write(r.stderr)
        raise SystemExit(f"failed ({r.returncode}): {' '.join(str(a) for a in args)}")


def main():
    check = "--check" in sys.argv
    for name in ("modeltool", "game1"):
        if "--build" in sys.argv or not program(name).exists():
            run([sys.executable, "-u", "build_rs.py", "app", name, "--target", TARGET])
    bad = []
    for jfs in sorted(CONTENT.glob("*.jfs")):
        out = OUT / jfs.stem
        run([program("modeltool"), jfs, "--out", out, "--force"])
        target = (OUT / "check" / (jfs.stem + ".g1m")) if check else (CONTENT / (jfs.stem + ".g1m"))
        target.parent.mkdir(parents=True, exist_ok=True)
        run([program("game1"), "--bake", out / "model.glb", target])
        if check:
            have = CONTENT / (jfs.stem + ".g1m")
            same = have.exists() and have.read_bytes() == target.read_bytes()
            print(f"{jfs.name}: {'the same bytes' if same else 'DIFFERENT from ' + str(have.relative_to(ROOT))}")
            if not same:
                bad.append(jfs.name)
    if bad:
        raise SystemExit(f"content differs: {', '.join(bad)} (python apps/game1/tools/content.py rebuilds it)")


if __name__ == "__main__":
    main()
