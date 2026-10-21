#!/usr/bin/env python3
"""fontdiff.py -- compare the Rust fonttest (build) with the assembly reference
(build/rwref, or jfgui_REF) on real fonts: every mode, byte for byte.

  python tools/fontdiff.py [--target win|linux|both] [--quick] [--fonts N]

Per font (system fonts of tools/font_check.py, CFF .otf files of the Fonts folder, the
fixtures): --dump (stdout), --outlines over every glyph id and a codepoint range (the record
file), --kern over Latin pairs (the record file), --render at several sizes / styles /
decorations (decoded pixels).  Also --fuzz, --scan (the timing on the first line masked),
--find and --selftest.  --cmpsgf (embedded fonts 0..2 vs the Selawik TTFs in
styles/winui/assets) runs on the Rust build only and must report no difference: a reference
built before the switch to Selawik still embeds Segoe UI, so its --cmpsgf and --render @0..@2
output differ from the Rust build by design.  Exit code 0 = no differences.  Outputs below
build/fontdiff/.
"""
import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import pngutil  # noqa: E402
from font_check import SGF_SOURCES, origin_fixed  # noqa: E402

REF = Path(os.environ.get("jfgui_REF") or ROOT / "build" / "rwref")
RS = ROOT / "build"
OUT = RS / "fontdiff"
WINFONTS = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts"
WSL = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")
FIX = ROOT / "apps" / "fonttest" / "tests" / "fixtures"

FONTS = [
    ("arial.ttf", 0), ("arialbd.ttf", 0), ("ariali.ttf", 0), ("times.ttf", 0), ("timesbi.ttf", 0),
    ("segoeui.ttf", 0), ("seguisb.ttf", 0), ("segoeuib.ttf", 0), ("consola.ttf", 0), ("calibri.ttf", 0),
    ("georgia.ttf", 0), ("verdana.ttf", 0), ("tahoma.ttf", 0), ("comic.ttf", 0), ("impact.ttf", 0),
    ("cambria.ttc", 0), ("cambria.ttc", 1), ("msgothic.ttc", 0), ("msgothic.ttc", 1), ("simsun.ttc", 0),
    ("malgun.ttf", 0), ("symbol.ttf", 0), ("wingding.ttf", 0), ("webdings.ttf", 0), ("seguisym.ttf", 0),
    ("seguiemj.ttf", 0), ("SegUIVar.ttf", 0), ("bahnschrift.ttf", 0), ("SegoeIcons.ttf", 0), ("framd.ttf", 0),
]
QUICK = {"arial.ttf", "segoeui.ttf", "times.ttf", "cambria.ttc", "seguiemj.ttf", "symbol.ttf", "SegUIVar.ttf"}
CFF_MAX = 8
KERN_CHARS = "AVTWYLOPFJkoaevwyr.,-"
TEXTS = ["Hamburgefonstiv ÄÖÜ äöü ß AVATAR fi 0123", "Wolf Ťý ĳ ﬁ “quoted” — € ¶ ∑ → ☺ 😀 中文"]
SIZES = [9, 11, 14, 16, 24, 48]
STYLES = [0, 1, 2, 3, 4, 7]


class Diff:
    def __init__(self):
        self.compared = 0
        self.diffs = 0

    def same(self, ok, what):
        self.compared += 1
        if not ok:
            self.diffs += 1
            if self.diffs <= 40:
                print(f"  DIFF {what}")


D = Diff()


def wsl_path(p):
    p = str(Path(p).resolve())
    if len(p) > 1 and p[1] == ":":
        return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/")
    return p.replace("\\", "/")


def run(exe_dir, target, args, timeout=900):
    """-> (exit code, stdout bytes); Path arguments are converted for WSL"""
    exe = exe_dir / target / ("fonttest.exe" if target == "win" else "fonttest")
    shot = OUT / "shot.png"
    if target == "win":
        cmd = [str(exe), "--shot", str(shot)] + [str(a) for a in args]
    else:
        cmd = ["wsl.exe", "-d", WSL, "--cd", str(ROOT), "-e", wsl_path(exe), "--shot", wsl_path(shot)]
        cmd += [wsl_path(a) if isinstance(a, Path) else str(a) for a in args]
    env = dict(os.environ)
    env.pop("jfgui_FONTDIRS", None)
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, timeout=timeout, env=env)
    return r.returncode, r.stdout or b""


def both(target, args, outfile=None, mask=None, fmask=None):
    """run the reference and the Rust build; compare exit code, stdout and outfile"""
    res = []
    for d in (REF, RS):
        if outfile:
            outfile.unlink(missing_ok=True)
        code, so = run(d, target, args)
        data = outfile.read_bytes() if outfile and outfile.exists() else None
        if fmask and data is not None:
            data = fmask(data)
        if mask:
            so = mask(so)
        res.append((code, so, data))
    (c1, s1, f1), (c2, s2, f2) = res
    what = f"[{target}] {' '.join(str(a) if not isinstance(a, Path) else a.name for a in args)[:150]}"
    D.same(c1 == c2, f"{what}: exit {c1} vs {c2}")
    D.same(s1 == s2, f"{what}: stdout differs\n    ref: {s1[:300]!r}\n    rs:  {s2[:300]!r}")
    if outfile:
        D.same(f1 == f2, f"{what}: {outfile.name} differs ({first_line_diff(f1, f2)})")
    return res


def first_line_diff(a, b):
    if a is None or b is None:
        return f"missing: ref {a is None}, rs {b is None}"
    la, lb = a.split(b"\n"), b.split(b"\n")
    for i, (x, y) in enumerate(zip(la, lb)):
        if x != y:
            return f"line {i + 1}: {x[:100]!r} vs {y[:100]!r}"
    return f"{len(la)} vs {len(lb)} lines"


def render(target, font, face, size, text, style, decor):
    args = ["--render", font, face, size, text]
    outs = []
    for d in (REF, RS):
        png = OUT / f"r_{d.name}.png"
        png.unlink(missing_ok=True)
        code, _ = run(d, target, args + [png] + (["--style", style] if style else []) +
                      (["--decor", decor] if decor else []))
        outs.append((code, pngutil.read_png(png) if png.exists() else None))
    what = f"[{target}] render {Path(str(font)).name} {face} {size} style {style} decor {decor} {text[:20]!r}"
    (c1, p1), (c2, p2) = outs
    D.same(c1 == c2 and p1 == p2, f"{what}: exit {c1}/{c2}, pixels {'equal' if p1 == p2 else 'differ'}")


def glyph_count(target, font, face):
    _, so = run(REF, target, ["--dump", font, face])
    for ln in so.decode("utf-8", "replace").splitlines():
        if ln.startswith("upem "):
            return int(ln.split()[3])
    return 0


def font_list(quick, limit):
    fl = [(WINFONTS / f, i) for f, i in FONTS if (WINFONTS / f).exists() and (not quick or f in QUICK)]
    cff = sorted(p for p in WINFONTS.glob("*.otf"))[:2 if quick else CFF_MAX]
    fl += [(p, 0) for p in cff]
    fl += [(FIX / f, i) for f, i in [("mini.ttf", 0), ("mini.ttc", 1), ("mini.otf", 0), ("mac.ttf", 0),
                                     ("cid.otf", 0), ("cbdt.ttf", 0)]]
    return fl[:limit] if limit else fl


def check_target(target, quick, limit):
    OUT.mkdir(parents=True, exist_ok=True)
    fonts = font_list(quick, limit)
    for font, face in fonts:
        print(f"[{target}] {font.name} {face}")
        both(target, ["--dump", font, face])
        n = glyph_count(target, font, face)
        if quick:
            n = min(n, 1500)
        lst = OUT / "list.txt"
        lines = [f"g{g}" for g in range(n)] + [f"u {cp}" for cp in list(range(0x20, 0x250)) + [0x20AC, 0x2192, 0x1F600]]
        lst.write_text("\n".join(lines) + "\n", encoding="utf-8")
        both(target, ["--outlines", font, face, lst, OUT / "ol.txt"], OUT / "ol.txt")
        kl = OUT / "kern.txt"
        kl.write_text("\n".join(f"{ord(a)} {ord(b)}" for a in KERN_CHARS for b in KERN_CHARS) + "\n", encoding="utf-8")
        both(target, ["--kern", font, face, kl, OUT / "k.txt"], OUT / "k.txt")
        sizes = [11, 16, 48] if quick else SIZES
        for size in sizes:
            for style in (STYLES if size == 16 else [0]):
                for text in TEXTS[:1] if quick else TEXTS:
                    render(target, font, face, size, text, style, 3 if style == 7 else 0)
    # embedded fonts
    for i in range(4):
        render(target, f"@{i}", 0, 14, "Hamburgefonstiv ÄÖÜ äöü ß AVATAR", 0, 0)
        render(target, f"@{i}", 0, 20, "Hamburgefonstiv AVATAR", 7, 3)
    for src, sid in SGF_SOURCES[:3]:
        if not quick:
            p, _ = origin_fixed(src, OUT)
            code, so = run(RS, target, ["--cmpsgf", p, 0, sid])
            last = (so.decode("utf-8", "replace").strip().splitlines() or ["?"])[-1]
            D.same(code == 0, f"[{target}] --cmpsgf {src.name} {sid} (Rust build): {last}")
    for f in ["mini.ttf", "mini.otf", "cid.otf", "cbdt.ttf", "mac.ttf"]:
        both(target, ["--fuzz", FIX / f])
    for f in (["arial.ttf", "seguiemj.ttf"] if quick else ["arial.ttf", "seguiemj.ttf", "cambria.ttc", "SegUIVar.ttf"]):
        if (WINFONTS / f).exists():
            both(target, ["--fuzz", WINFONTS / f])

    def mask_time(so):
        ls = so.split(b"\n", 1)
        if ls and ls[0].startswith(b"# "):
            ls[0] = b" ".join(ls[0].split(b" ")[:5])
        return b"\n".join(ls)

    both(target, ["--scan", OUT / "scan.txt", "--rescan"], OUT / "scan.txt", fmask=mask_time)
    for fam, w, it in [("Segoe UI", 400, 0), ("Segoe UI", 700, 1), ("Arial", 400, 1), ("Times New Roman", 700, 1),
                       ("Consolas", 400, 0), ("Segoe UI Variable", 600, 0), ("Cambria Math", 400, 0),
                       ("nope", 400, 0)]:
        both(target, ["--find", fam, w, it])
    both(target, ["--selftest"])


def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--fonts", type=int, default=0, help="only the first N fonts")
    a = ap.parse_args()
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        check_target(t, a.quick, a.fonts)
    print(f"fontdiff: {D.compared} comparisons, {D.diffs} differences")
    return 1 if D.diffs else 0


if __name__ == "__main__":
    sys.exit(main())
