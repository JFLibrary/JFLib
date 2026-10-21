#!/usr/bin/env python3
"""clip_rs_check.py -- the clipboard image conversions of plat (clipdrop.rs: clip_dib_to_bmp,
clip_bmp_fix) against the assembly, the parts of tools/clip_check.py that need no PaintCraft
(agent plat-shared, R4).

  python tools/platshared/clip_rs_check.py [--target win|linux|both] [--ref build/rwref] [--rs build]

Every DIB variant of clip_check.py (BITMAPCOREHEADER / INFO / V4 / V5, bottom-up / top-down,
32 bit BI_RGB / BI_BITFIELDS / BI_ALPHABITFIELDS, 24 bit, 8 bit palette, paint.net's alpha
rule) goes through the script commands "clip dib F" (the CF_DIB path) and its BMP files
through "clip file F" (the image/bmp path) of the template app, then "clip save OUT bmp":
the assembly's template and the Rust one must write the same BMP bytes, and the BMP decoded
here must hold clip_check's expected pixels.  The invalid DIBs must be refused by both
("invalid DIB / BMP data").  Only the in-process clipboard of --script is used; the real
clipboard is never touched.  Exit code 0 = passed.
"""
import argparse, struct, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import clip_check as cc  # noqa: E402

WSL = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e"]
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1


def run(target, exe, script):
    args = ["--script", script]
    cmd = [str(ROOT / (exe + ".exe"))] + args if target == "win" else WSL + ["./" + exe] + args
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
    return r.returncode, r.stdout + r.stderr


def bmp_decode(b):
    """-> rows of (a, r, g, b) (alpha 255 where the format has none / the alpha mask is 0)"""
    off, = struct.unpack_from("<I", b, 10)
    hsz, = struct.unpack_from("<I", b, 14)
    if hsz == 12:
        w, h, _, bpp = struct.unpack_from("<HHHH", b, 18)
        comp, clr = 0, 0
    else:
        w, h, _, bpp, comp = struct.unpack_from("<iiHHI", b, 18)
        clr, = struct.unpack_from("<I", b, 46)
    top = h < 0
    h = abs(h)
    masks = None
    if bpp == 32 and comp in (3, 6):
        masks = struct.unpack_from("<IIII", b, 54) if hsz >= 56 else struct.unpack_from("<IIII", b, 14 + hsz)
    pal = []
    if bpp == 8:
        n = clr or 256
        ps = 14 + hsz
        pal = [b[ps + i * 4:ps + i * 4 + 4] for i in range(n)]
    stride = (w * bpp + 31) // 32 * 4

    def chan(v, m):
        if m == 0:
            return None
        sh = (m & -m).bit_length() - 1
        return (v & m) >> sh
    out = []
    for y in range(h):
        row = b[off + (y if top else h - 1 - y) * stride:]
        px = []
        for x in range(w):
            if bpp == 32:
                v, = struct.unpack_from("<I", row, x * 4)
                if masks:
                    r, g, bb, a = (chan(v, m) for m in masks)
                    px.append((255 if a is None else a, r, g, bb))
                else:
                    px.append((255, v >> 16 & 255, v >> 8 & 255, v & 255))
            elif bpp == 24:
                bb, g, r = row[x * 3:x * 3 + 3]
                px.append((255, r, g, bb))
            else:
                e = pal[row[x]]
                px.append((255, e[2], e[1], e[0]))
        out.append(px)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--ref", default="build/rwref")
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    ref, rs = a.ref.rstrip("/"), a.rs.rstrip("/")
    work = ROOT / "build" / "plat-shared" / "clip"
    work.mkdir(parents=True, exist_ok=True)
    wrel = "build/plat-shared/clip"
    s, z = cc.source(), cc.with_alpha(cc.source(), 0)
    cases = [(n, "dib", d, e) for n, d, e in cc.dib_variants()]
    cases += [("bmp_v5_alpha", "file", cc.bmp_file(cc.info_header(32, 3, False, 124, cc.STD) + cc.rows32(s, False), 124), s),
              ("bmp_v5_alpha0", "file", cc.bmp_file(cc.info_header(32, 3, False, 124, cc.STD) + cc.rows32(z, False), 124), cc.opaque(s)),
              ("bmp_info32_rgb", "file", cc.bmp_file(cc.info_header(32, 0, True) + cc.rows32(s, True), 40), s),
              ("bmp_24", "file", cc.bmp_file(cc.info_header(24, 0, False) + cc.rows24(s, False), 40), cc.opaque(s))]
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        for name, how, data, exp in cases:
            src = work / f"{name}.{'dib' if how == 'dib' else 'bmp'}"
            src.write_bytes(data)
            outs = {}
            for impl, base in (("asm", ref), ("rs", rs)):
                o = work / f"{t}_{impl}_{name}.out.bmp"
                if o.exists():
                    o.unlink()
                scr = work / f"{t}_{impl}_{name}.uis"
                scr.write_text(f"size 640 480\nclip {how} {wrel}/{src.name}\nclip save {wrel}/{o.name} bmp\n",
                               encoding="utf-8", newline="\n")
                rc, out = run(t, f"{base}/{t}/template", f"{wrel}/{scr.name}")
                outs[impl] = (rc, o.read_bytes() if o.exists() else b"", out)
            (ra, ba, oa), (rr, br, orr) = outs["asm"], outs["rs"]
            check(f"[{t}] {name}: both converted (exit 0)", ra == 0 and rr == 0 and ba and br, (oa[-300:], orr[-300:]))
            check(f"[{t}] {name}: the BMP byte for byte as the assembly's ({len(br)} bytes)", ba == br, (len(ba), len(br)))
            if br:
                got = bmp_decode(br)
                check(f"[{t}] {name}: the expected pixels (paint.net's alpha rule)", got == exp)
        for name, data in cc.invalid_variants():
            src = work / f"{name}.dib"
            src.write_bytes(data)
            res = {}
            for impl, base in (("asm", ref), ("rs", rs)):
                scr = work / f"{t}_{impl}_{name}.uis"
                scr.write_text(f"size 640 480\nclip dib {wrel}/{src.name}\nclip expect none\n", encoding="utf-8", newline="\n")
                res[impl] = run(t, f"{base}/{t}/template", f"{wrel}/{scr.name}")
            (ra, oa), (rr, orr) = res["asm"], res["rs"]
            ok = all(rc == 1 and "invalid DIB" in o and o.count("FAIL") == 1 for rc, o in ((ra, oa), (rr, orr)))
            check(f"[{t}] {name}: refused by both", ok, (oa.strip()[-200:], orr.strip()[-200:]))
    print("clip_rs_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
