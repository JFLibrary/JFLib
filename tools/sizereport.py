"""Binary size report: code / data per library layer and app module.

    python tools/sizereport.py [--target win|linux] [--app Kritzel] [--md]

Reads the object files that build.py left in build/<target>/obj and the final
executables. Sizes come from `llvm-size -A`; `.rodata` of text/font.* contains the
embedded font / icon outlines and is reported separately as "assets".
"""
import argparse
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OBJ_EXT = {"win": ".obj", "linux": ".o"}
EXE = {"win": "{}.exe", "linux": "{}"}


def sections(path):
    out = subprocess.run(["llvm-size", "-A", path], capture_output=True, text=True).stdout
    res = {"text": 0, "rodata": 0, "data": 0, "bss": 0}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 2 or not parts[1].isdigit():
            continue
        name, size = parts[0], int(parts[1])
        if name.startswith(".text"):
            res["text"] += size
        elif name.startswith(".rodata") or name.startswith(".rdata"):
            res["rodata"] += size
        elif name.startswith(".data"):
            res["data"] += size
        elif name.startswith(".bss"):
            res["bss"] += size
    return res


def kb(n):
    return f"{n / 1024:8.1f}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="win", choices=["win", "linux"])
    ap.add_argument("--app", action="append", help="app(s) to list per module (default: all built)")
    ap.add_argument("--md", action="store_true", help="markdown table output")
    a = ap.parse_args()

    base = os.path.join(ROOT, "build", a.target, "obj")
    ext = OBJ_EXT[a.target]
    if not os.path.isdir(base):
        sys.exit(f"no objects in {base}; run python build.py all --target {a.target} first")

    rows = []                    # (group, module, sections)
    for dirpath, _, files in os.walk(base):
        for f in sorted(files):
            if not f.endswith(ext):
                continue
            rel = os.path.relpath(os.path.join(dirpath, f), base).replace("\\", "/")
            parts = rel.split("/")
            if parts[0] == "asmlib":
                group = "lib/" + parts[1]
            elif parts[0] == "styles":
                group = "style/" + parts[1]
            elif parts[0] == "apps":
                if a.app and parts[1] not in a.app:
                    continue
                group = "app/" + parts[1]
            else:
                group = parts[0]
            rows.append((group, parts[-1][: -len(ext)], sections(os.path.join(dirpath, f))))

    groups = {}
    assets = 0
    for g, m, s in rows:
        t = groups.setdefault(g, {"text": 0, "rodata": 0, "data": 0, "bss": 0, "n": 0})
        rod = s["rodata"]
        if g.startswith("style/") and m == "fonts":
            assets += rod
            rod = 0
        for k in ("text", "data", "bss"):
            t[k] += s[k]
        t["rodata"] += rod
        t["n"] += 1

    if a.md:
        print("| Gruppe | Module | Code KB | Konst. KB | Daten KB | BSS KB |")
        print("|---|---:|---:|---:|---:|---:|")
        for g in sorted(groups):
            t = groups[g]
            print(f"| {g} | {t['n']} | {t['text']/1024:.1f} | {t['rodata']/1024:.1f} | "
                  f"{t['data']/1024:.1f} | {t['bss']/1024:.1f} |")
    else:
        print(f"{'group':<18}{'mods':>5}{'code KB':>9}{'const KB':>9}{'data KB':>9}{'bss KB':>9}")
        for g in sorted(groups):
            t = groups[g]
            print(f"{g:<18}{t['n']:>5}{kb(t['text'])}{kb(t['rodata'])}{kb(t['data'])}{kb(t['bss'])}")
    lib_code = sum(t["text"] for g, t in groups.items() if g.startswith("lib/"))
    lib_const = sum(t["rodata"] for g, t in groups.items() if g.startswith("lib/"))
    print()
    print(f"library code {lib_code / 1024:.1f} KB, constants {lib_const / 1024:.1f} KB, "
          f"embedded font/icon assets {assets / 1024:.1f} KB")
    for app in sorted({g[4:] for g in groups if g.startswith("app/")}):
        exe = os.path.join(ROOT, "build", a.target, EXE[a.target].format(app))
        if os.path.exists(exe):
            print(f"{app}: executable {os.path.getsize(exe) / 1024:.1f} KB")


if __name__ == "__main__":
    main()
