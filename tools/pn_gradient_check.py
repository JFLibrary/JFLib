#!/usr/bin/env python3
"""pn_gradient_check.py -- PaintCraft Farbverlauf tool (apps/PaintCraft/src/t_gradient.s, agent T3)
against a Python reference, bit exact.

A 200 x 150 RGBA fixture (varying colour and alpha) is opened; for every case the script sets
the options / colours through the test hooks (792 type | repeat << 8 | transparency << 16,
793 start, 794 end, 943 / 944 test colours, 932 antialiasing, 933 blend mode), commits the
live gradient (Enter), compares the FNV-1a 64 hash of the whole layer (hook 995 0 -> #9990)
with the reference and undoes the step.  Reference (tools/gradient_check.py formulas):
  colour mode: T of grad_t_span, c = (A (16384 - T) + B T + 8192) >> 14 per channel; opaque
    colours with Normal (= the gradient colour) or any colours with Überschreiben (= exactly
    the gradient colour); right button = colours swapped;
  transparency mode: ramp primary.a -> 255 - secondary.a (right button reversed),
    alpha' = ((a ramp + 128) 257) >> 16, colours kept;
  antialiasing (only zyklische Wiederholung and spirals): 4 rotated-grid samples
    (+-0.375 / +-0.125), 4 equal samples copied, else a = (sa + 2) >> 2,
    c = (sum c_k a_k + sa / 2) // sa (0 when sa = 0); transparency: ramp = (sum + 2) >> 2.

usage: python tools/pn_gradient_check.py [--target win|linux|both] [--out build/t3]
"""
import argparse
import struct
import subprocess
import sys
import zlib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from gradient_check import ref_T  # noqa: E402

W, H = 200, 150
COLORS = [0x00000000, 0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0x80FF0000,
          0xFFFFFFFF, 0xFF000000, 0xFF808080, 0x80000000, 0xFFFFFF00]
SS = [(-0.375, -0.125), (0.125, -0.375), (0.375, 0.125), (-0.125, 0.375)]


def fixture(W=W, H=H):
    y, x = np.mgrid[0:H, 0:W]
    a = (x * 5 + y * 3) % 256
    a[:, :20] = 255
    px = np.stack([x % 256, y % 256, np.full_like(x, 128), a], -1).astype(np.uint8)  # R G B A
    raw = b"".join(b"\0" + px[j].tobytes() for j in range(H))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    argb = (px[..., 3].astype(np.uint32) << 24) | (px[..., 0].astype(np.uint32) << 16) | \
        (px[..., 1].astype(np.uint32) << 8) | px[..., 2].astype(np.uint32)
    return png, argb


def split(c):
    return np.array([(c >> 16) & 255, (c >> 8) & 255, c & 255, c >> 24], np.int64)   # R G B A


def lerp_rgba(T, A, B):
    return (A[None, None, :] * (16384 - T)[..., None] + B[None, None, :] * T[..., None] + 8192) >> 14


def samples(typ, rep, pts, aa, W=W, H=H):
    sx, sy, ex, ey = pts
    if not aa:
        return [ref_T(W, H, 0, 0, typ, rep, sx, sy, ex, ey)]
    return [ref_T(W, H, 0, 0, typ, rep, sx - ox, sy - oy, ex - ox, ey - oy) for ox, oy in SS]


def expected(base, c):
    typ, rep, alpha, ca, cb, s, e, aa, blend, right = c
    pts = (s[0] + 0.5, s[1] + 0.5, e[0] + 0.5, e[1] + 0.5)
    use_aa = aa and (rep == 1 or typ >= 5)
    Ts = samples(typ, rep, pts, use_aa, base.shape[1], base.shape[0])
    P, S = COLORS[ca], COLORS[cb]
    if alpha:
        a0, a1 = P >> 24, 255 - (S >> 24)
        if right:
            a0, a1 = a1, a0
        ramps = [(a0 * (16384 - T) + a1 * T + 8192) >> 14 for T in Ts]
        ramp = ramps[0] if len(ramps) == 1 else (sum(ramps) + 2) >> 2
        a = (base >> 24).astype(np.int64)
        na = ((a * ramp + 128) * 257) >> 16
        return (base & 0x00FFFFFF) | (na.astype(np.uint32) << 24)
    A, B = split(P), split(S)
    if right:
        A, B = B, A
    cols = [lerp_rgba(T, A, B) for T in Ts]
    if len(cols) == 1:
        out = cols[0]
    else:
        st = np.stack(cols)                                       # 4 x H x W x 4
        same = np.all(st == st[0:1], axis=(0, 3))
        sa = st[..., 3].sum(0)
        num = (st[..., :3] * st[..., 3:4]).sum(0)
        with np.errstate(divide="ignore", invalid="ignore"):
            cc = np.where(sa[..., None] > 0, (num + (sa[..., None] >> 1)) // np.maximum(sa[..., None], 1), 0)
        mixed = np.concatenate([cc, ((sa + 2) >> 2)[..., None]], -1)
        mixed = np.where(sa[..., None] > 0, mixed, 0)
        out = np.where(same[..., None], st[0], mixed)
    o = out.astype(np.uint32)
    grad = (o[..., 3] << 24) | (o[..., 0] << 16) | (o[..., 1] << 8) | o[..., 2]
    assert blend == 14 or ((P >> 24) == 255 and (S >> 24) == 255), "Normal needs opaque colours"
    return grad


def fnv(argb):
    h = 0xCBF29CE484222325
    for b in argb.astype("<u4").tobytes():
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "%016x" % h


def cases():
    out = []
    geo = [((20, 30), (170, 110)), ((150, 20), (60, 140)), ((100, 75), (130, 75)), ((10, 140), (10, 20))]
    k = 0
    for typ in range(7):
        for rep in range(3):
            s, e = geo[k % len(geo)]
            k += 1
            out.append((typ, rep, 0, 1, 3, s, e, 0, 0, 0))                 # red -> blue, Normal
            out.append((typ, rep, 1, 8, 5, e, s, 0, 0, 0))                 # 50 % -> 0 transparency
            out.append((typ, rep, 0, 4, 2, s, e, 0, 14, k & 1))            # alpha colours, Überschreiben
    for typ in range(7):                                                    # antialiasing
        out.append((typ, 1, 0, 1, 3, (40, 40), (90, 60), 1, 0, 0))
        out.append((typ, 1, 0, 4, 2, (100, 75), (120, 95), 1, 14, 0))
    for typ in (5, 6):
        out.append((typ, 0, 0, 6, 9, (100, 75), (140, 75), 1, 0, 1))
        out.append((typ, 2, 1, 6, 5, (100, 75), (100, 115), 1, 0, 0))
    out.append((3, 1, 1, 6, 5, (60, 60), (80, 80), 1, 0, 1))
    out.append((0, 0, 1, 6, 5, (0, 0), (199, 149), 0, 0, 1))               # right: reversed ramp
    out.append((0, 0, 0, 1, 3, (0, 0), (199, 0), 0, 0, 1))                 # right: blue -> red
    out.append((3, 0, 0, 1, 1, (50, 50), (50, 50), 0, 0, 0))               # degenerate: colour B
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/t3")
    a = ap.parse_args()
    out = ROOT / a.out
    (out / "pngc").mkdir(parents=True, exist_ok=True)
    png, base = fixture()
    fx = out / "pngc" / "grad_fixture.png"
    fx.write_bytes(png)
    lines = ["size 1280 800", "key g", "expect #9931 value 9", "cmd 995 0",
             'expect #9990 text "%s"' % fnv(base)]
    cs = cases()
    for i, c in enumerate(cs):
        typ, rep, alpha, ca, cb, s, e, aa, blend, right = c
        lines += ["# case %d %r" % (i, c), "cmd 943 %d" % ca, "cmd 944 %d" % cb, "cmd 932 %d" % aa,
                  "cmd 933 %d" % blend, "cmd 792 %d" % (typ | rep << 8 | alpha << 16),
                  "cmd 793 %d" % (s[0] | s[1] << 16 | (1 << 31 if right else 0)),
                  "cmd 794 %d" % (e[0] | e[1] << 16), "expect #524288 value 2", "key enter",
                  "expect #524288 value 0", "cmd 995 0",
                  'expect #9990 text "%s"' % fnv(expected(base, c)), "key ctrl+z"]
    lines += ["cmd 995 0", 'expect #9990 text "%s"' % fnv(base)]
    # a larger image (19 bands of 64 rows on the worker pool)
    bpng, bbase = fixture(1100, 1200)
    bfx = out / "pngc" / "grad_fixture_big.png"
    bfx.write_bytes(bpng)
    big = [(5, 1, 0, 1, 3, (500, 600), (560, 640), 1, 0, 0), (3, 2, 1, 6, 5, (1000, 100), (700, 900), 1, 0, 1)]
    blines = ["size 1280 800", "key g"]
    for c in big:
        typ, rep, alpha, ca, cb, s, e, aa, blend, right = c
        blines += ["cmd 943 %d" % ca, "cmd 944 %d" % cb, "cmd 932 %d" % aa, "cmd 933 %d" % blend,
                   "cmd 792 %d" % (typ | rep << 8 | alpha << 16),
                   "cmd 793 %d" % (s[0] | s[1] << 16 | (1 << 31 if right else 0)),
                   "cmd 794 %d" % (e[0] | e[1] << 16), "key enter", "cmd 995 0",
                   'expect #9990 text "%s"' % fnv(expected(bbase, c)), "key ctrl+z"]
    bscr = out / "pngc" / "pn_gradient_big.uis"
    bscr.write_text("\n".join(blines) + "\n", encoding="utf-8")
    scr = out / "pngc" / "pn_gradient_check.uis"
    scr.write_text("\n".join(lines) + "\n", encoding="utf-8")
    rel_fx = fx.relative_to(ROOT).as_posix()
    rel_scr = scr.relative_to(ROOT).as_posix()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    bad = 0
    runs = [(rel_fx, rel_scr, [], "%d cases" % len(cs)),
            (bfx.relative_to(ROOT).as_posix(), bscr.relative_to(ROOT).as_posix(), [], "1100 x 1200"),
            (bfx.relative_to(ROOT).as_posix(), bscr.relative_to(ROOT).as_posix(), ["--threads", "1"],
             "1100 x 1200 --threads 1")]
    for t in targets:
        for fxp, scp, extra, name in runs:
            if t == "win":
                cmd = [str(out / "win" / "PaintCraft.exe")]
            else:
                cmd = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e",
                       "./%s/linux/PaintCraft" % Path(a.out).as_posix()]
            cmd += extra + ["--open", fxp, "--script", scp]
            r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
            fails = [ln for ln in (r.stdout + r.stderr).splitlines() if "FAIL" in ln]
            print("[%s] %s: %s" % (t, name, "ok" if r.returncode == 0 else "FAILED (exit %d)" % r.returncode))
            for ln in fails[:20]:
                print("   ", ln)
            bad += r.returncode != 0
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
