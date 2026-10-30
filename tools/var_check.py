#!/usr/bin/env python3
"""var_check.py -- variable font instances of text/var.rs against HarfBuzz and FreeType.

The reference runs in WSL (Ubuntu-24.04) in the Flatpak runtime org.freedesktop.Sdk//26.08
(its python3 with ctypes on libharfbuzz and libfreetype; nothing is installed): tools/
var_ref_sdk.py loads the font there, sets the axis values (hb_font_set_variations,
FT_Set_Var_Design_Coordinates) and writes the normalized coordinates, every glyph's outline
(FreeType: FT_Load_Glyph NO_SCALE points, tags, contour ends; HarfBuzz: hb_font_draw_glyph
segments), its advance (both) and hb_shape of a text.  This side asks fonttest (--dump,
--outlines, --advances with --axes) for the same and compares:

  coordinates   equal to HarfBuzz's
  outlines      the points against FreeType (it rounds deltas to whole units: up to ~1.5 off
                after composites), the draw segments against HarfBuzz (float: 0.02)
  advances      every glyph against HarfBuzz and FreeType
  widths        the HarfRust-shaped run (Text::set_shaping) against hb_shape, and the plain run

  python tools/var_check.py                      the fonts below that exist on this machine
  python tools/var_check.py NAME...              some of them (bahn, segoe, cascadia, sitka,
                                                 ubuntu, var, phantom)
  python tools/var_check.py --font F --text T SPEC...   any font ("-": the default instance)
  --exe build/win/fonttest.exe (default)

Windows only (fonttest.exe, WSL); fonts are read in place (C:/Windows/Fonts is mounted into
the sandbox read-only).  Intermediate files: build/var_check/.
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TMP = ROOT / "build" / "var_check"
LOCAL = os.environ.get("LOCALAPPDATA", "").replace("\\", "/") + "/Microsoft/Windows/Fonts"
VF = (ROOT / "apps" / "fonttest" / "tests" / "varfont").as_posix()
CASES = {
    "bahn": ("C:/Windows/Fonts/bahnschrift.ttf", "AVATAR Wolf Typo",
             ["-", "wght=600", "wght=300,wdth=75", "wght=700,wdth=87.5", "wght=451.5,wdth=91", "wght=333.3,wdth=99.99"]),
    "segoe": ("C:/Windows/Fonts/SegUIVar.ttf", "Hamburgefonstiv AVATAR", ["-", "wght=650", "wght=300,opsz=5", "wght=512.25,opsz=20"]),
    "cascadia": ("C:/Windows/Fonts/CascadiaCode.ttf", "fn main() -> i32 { 0 }", ["-", "wght=200", "wght=555"]),
    "sitka": ("C:/Windows/Fonts/SitkaVF-Italic.ttf", "Hamburgefonstiv", ["-", "opsz=6,wght=700", "opsz=27.5,wght=444"]),
    "ubuntu": (LOCAL + "/UbuntuSansMono[wght].ttf", "Hamburgefonstiv", ["-", "wght=100", "wght=650"]),
    "var": (VF + "/jfvar.ttf", "A B.A VO", ["-", "wght=900", "wght=650", "wght=100,wdth=75", "wdth=87.5,slnt=-6,TEST=10",
                                             "wght=250,wdth=50,slnt=-12", "wght=777,wdth=61.3,slnt=-3.3,TEST=2.2"]),
    "phantom": (VF + "/jfvar-phantom.ttf", "A a. A", ["-", "wght=800", "wght=300", "wght=523.7", "wght=211", "wght=600,opsz=30",
                                                      "opsz=47,wght=777"]),
}


def wsl(p):
    p = Path(p).resolve().as_posix()
    return "/mnt/" + p[0].lower() + p[2:] if p[1:2] == ":" else p


def fonttest(exe, args):
    return subprocess.run([exe] + args, capture_output=True, text=True, encoding="utf-8", errors="replace").stdout


def read_outlines(path):
    """fonttest --outlines -> {gid: {adv, pts [[x, y, tag]], ends}}"""
    out, cur = {}, None
    for ln in Path(path).read_text().splitlines():
        p = ln.split()
        if p[0] == "G":
            cur = {"adv": int(p[3]), "pts": [], "ends": []}
            out[int(p[2])] = cur
        elif p[0] == "P":
            cur["pts"].append([int(p[2]) / 1024, int(p[3]) / 1024, int(p[1])])
        elif p[0] == "E":
            cur["ends"].append(len(cur["pts"]) - 1)
    return out


def hb_path(pts, ends):
    """HarfBuzz's glyf path builder (quadratic contours) over a point list"""
    segs, start = [], 0
    for e in ends:
        fon = foff = loff = None
        cont = pts[start:e + 1]
        if cont and not cont[0][2] & 1:
            # an off-curve start: HarfBuzz begins at the last point (on) or at the midpoint
            last = cont[-1]
            if last[2] & 1:
                cont = [last] + cont[:-1]
            else:
                cont = [((last[0] + cont[0][0]) * .5, (last[1] + cont[0][1]) * .5, 1)] + cont
        for x, y, t in cont:
            p = (x, y)
            if fon is None:
                if t & 1:
                    fon = p
                    segs.append(["M", *p])
                elif foff is not None:
                    fon = ((foff[0] + p[0]) * .5, (foff[1] + p[1]) * .5)
                    loff = p
                    segs.append(["M", *fon])
                else:
                    foff = p
            elif loff is not None:
                if t & 1:
                    segs.append(["Q", *loff, *p])
                    loff = None
                else:
                    mid = ((loff[0] + p[0]) * .5, (loff[1] + p[1]) * .5)
                    segs.append(["Q", *loff, *mid])
                    loff = p
            elif t & 1:
                segs.append(["L", *p])
            else:
                loff = p
        if foff is not None and loff is not None:
            segs.append(["Q", *loff, (loff[0] + foff[0]) * .5, (loff[1] + foff[1]) * .5])
            loff = None
        if foff is not None and fon is not None:
            segs.append(["Q", *foff, *fon])
        elif loff is not None and fon is not None:
            segs.append(["Q", *loff, *fon])
        elif fon is not None:
            segs.append(["L", *fon])
        segs.append(["Z"])
        start = e + 1
    return segs


def seg_diff(a, b):
    if len(a) != len(b):
        return None
    m = 0.0
    for s, t in zip(a, b):
        if s[0] != t[0] or len(s) != len(t):
            return None
        m = max([m] + [abs(u - v) for u, v in zip(s[1:], t[1:])])
    return m


def check(exe, font, text, spec):
    """one instance -> (ok, line, details)"""
    ax = "" if spec == "-" else spec
    opt = ["--axes", ax] if ax else []
    dump = fonttest(exe, ["--dump", font, "0"] + opt)
    ng = int(next(ln for ln in dump.splitlines() if ln.startswith("upem")).split()[3])
    coords = next((ln.split()[1:] for ln in dump.splitlines() if ln.startswith("coords")), [])
    lst, mine_f, ref_f = TMP / "list.txt", TMP / "mine.txt", TMP / "ref.json"
    lst.write_text("".join(f"g{g}\n" for g in range(ng)))
    fonttest(exe, ["--outlines", font, "0", str(lst), str(mine_f)] + opt)
    adv = fonttest(exe, ["--advances", font, "0", text] + opt)
    mine = read_outlines(mine_f)
    sh = TMP / "ref.sh"
    wfont = wsl(font)
    sh.write_bytes((f"flatpak run --filesystem={wsl(ROOT)}:ro --filesystem={wsl(TMP)} --filesystem={wfont.rsplit('/', 1)[0]}:ro "
                    f"--command=python3 org.freedesktop.Sdk//26.08 -I {wsl(ROOT / 'tools' / 'var_ref_sdk.py')} "
                    f"'{wfont}' 0 '{ax}' all '{text}' {wsl(ref_f)}\n").encode())
    r = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--", "bash", wsl(sh)], capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    if "ref ok" not in r.stdout:
        return False, f"[{spec}] the reference failed: {(r.stdout + r.stderr).strip()[-500:]}", []
    ref = json.loads(ref_f.read_text())
    my_coords = [int(c) for c in coords] or [0] * len(ref["hb_coords"])
    ft_max = hb_max = 0.0
    bad, notes = 0, []
    for g in range(ng):
        m, rg = mine[g], ref["glyphs"][str(g)]
        f = rg.get("ft")
        if f is not None:
            if [p[2] & 1 for p in m["pts"]] != [p[2] & 1 for p in f["pts"]] or m["ends"] != f["ends"]:
                bad += 1
                notes.append(f"g{g}: other points than FreeType ({len(m['pts'])} / {len(f['pts'])})")
            else:
                ft_max = max([ft_max] + [max(abs(a[0] - b[0]), abs(a[1] - b[1])) for a, b in zip(m["pts"], f["pts"])])
            if m["adv"] != f["adv"]:
                notes.append(f"g{g}: advance {m['adv']}, FreeType {f['adv']}")
        d = seg_diff(hb_path(m["pts"], m["ends"]), rg["hb"]["segs"])
        if d is None or d > 0.02:
            bad += 1
            notes.append(f"g{g}: outline differs from HarfBuzz ({d})")
        else:
            hb_max = max(hb_max, d)
        if m["adv"] != rg["hb"]["adv"]:
            bad += 1
            notes.append(f"g{g}: advance {m['adv']}, HarfBuzz {rg['hb']['adv']}")
    w = next(ln for ln in adv.splitlines() if ln.startswith("W")).split()
    hb_w = sum(s[1] for s in ref["shape"])
    ok = bad == 0 and my_coords == ref["hb_coords"] and float(w[2]) == hb_w
    line = (f"[{spec}] coords {my_coords} {'=' if my_coords == ref['hb_coords'] else '!= ' + str(ref['hb_coords'])} HarfBuzz; "
            f"{ng} glyphs: HarfBuzz outlines max {hb_max:.4f}, FreeType points max {ft_max:.3f}, {bad} differ; "
            f"shaped {w[2]} hb_shape {hb_w} plain {w[1]}")
    return ok, line, notes


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("names", nargs="*", help="cases (default: all that exist) or, with --font, SPECs")
    ap.add_argument("--font")
    ap.add_argument("--text", default="AVATAR Hamburgefonstiv")
    ap.add_argument("--exe", default=str(ROOT / "build" / "win" / "fonttest.exe"))
    a = ap.parse_args()
    TMP.mkdir(parents=True, exist_ok=True)
    runs = [(a.font, a.text, a.names or ["-"])] if a.font else [CASES[n] for n in (a.names or CASES)]
    fails = 0
    for font, text, specs in runs:
        if not Path(font).exists():
            print(f"{font}: missing, skipped")
            continue
        print(font)
        for spec in specs:
            ok, line, notes = check(a.exe, font, text, spec)
            fails += not ok
            print(("  " if ok else "  FAIL ") + line)
            for n in notes[:6]:
                print("      " + n)
    print("var_check: " + ("ok" if not fails else f"{fails} instance(s) differ"))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
