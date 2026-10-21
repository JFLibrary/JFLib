#!/usr/bin/env python3
"""gradient_check.py -- check gfx/gradient.s (grad_span, grad_alpha_span) against numpy.

  python tools/gradient_check.py [--target win|linux|both] [--out build/g2] [--bench] [--keep]

The reference replicates the f64 operation order of the assembly (see the header of
ASMLIB/src/gfx/gradient.s), so the positions T and the colours are expected to match
exactly; the check allows |d| <= 1.  Cases: all 7 types x 3 repeat modes x colour /
dither / transparency (alpha multiply) / A8 modes, several geometries (short and long,
diagonal, start outside the image, negative coordinates), odd widths and chunk borders,
the degenerate start == end, and the atan2 approximation error (< 1/1024 turn required).
--bench reports Mpx/s per type (4000 x 3000, colour mode).
"""

import argparse
import math
import shutil
import struct
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from resample_check import FMT_ARGB, FMT_A8, split, join, write_raw, read_raw, Runner, us_of  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TYPES = ["linear", "linear_reflected", "diamond", "radial", "conical", "spiral_cw", "spiral_ccw"]
REPEATS = ["none", "wrap", "reflect"]
GF_ALPHA_MODE, GF_DITHER = 1, 2
AT = [0.9998660, -0.3302995, 0.1801410, -0.0851330, 0.0208351]
DBL_MIN = sys.float_info.min
BAYER = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.int64) * 1024 + 512


def hx(v):
    return "%016X" % struct.unpack("<Q", struct.pack("<d", float(v)))[0]


def atan2a(y, x):
    ax, ay = np.abs(x), np.abs(y)
    mx = np.maximum(np.maximum(ax, ay), DBL_MIN)
    z = np.minimum(ax, ay) / mx
    s = z * z
    p = AT[4]
    for c in (AT[3], AT[2], AT[1], AT[0]):
        p = p * s + c
    p = p * z
    p = np.where(ax < ay, math.pi / 2 - p, p)
    p = np.where(x < 0, math.pi - p, p)
    return np.where(y < 0, -p, p)


def frac(v):
    v = np.clip(v, -2.0 ** 30, 2.0 ** 30)
    return v - np.floor(v)


def ref_T(W, H, x0, y0, typ, rep, sx, sy, ex, ey):
    dx = ex - sx
    dy = ey - sy
    L2 = dx * dx + dy * dy
    if not (L2 > 0) or not math.isfinite(L2):
        return np.full((H, W), 16384, np.int64)
    invL2 = 1.0 / L2
    invL = 1.0 / math.sqrt(L2)
    T = np.zeros((H, W), np.int64)
    xs = np.arange(x0, x0 + W, dtype=np.float64)
    vx = (xs + 0.5) - sx
    for j in range(H):
        vy = (float(y0 + j) + 0.5) - sy
        cdot, ccross, cvv = vy * dy, vy * dx, vy * vy
        dot = vx * dx + cdot
        cross = ccross - vx * dy
        if typ == 0:
            t = dot * invL2
        elif typ == 1:
            t = np.abs(dot) * invL2
        elif typ == 2:
            t = (np.abs(dot) + np.abs(cross)) * invL2
        elif typ == 3:
            t = np.sqrt(vx * vx + cvv) * invL
        else:
            r = np.sqrt(vx * vx + cvv) * invL
            th = atan2a(cross, dot)
            if typ == 4:
                t = np.abs(th) * (1 / math.pi)
            elif typ == 5:
                t = frac(th * (1 / (2 * math.pi)) + r)
            else:
                t = frac(-(th * (1 / (2 * math.pi))) + r)
        if rep == 1:
            t = frac(t)
        elif rep == 2:
            fr = frac(t * 0.5)
            t = 1.0 - np.abs((fr + fr) - 1.0)
        else:
            t = np.minimum(np.maximum(t, 0.0), 1.0)
        T[j] = np.floor(t * 16384.0 + 0.5).astype(np.int64)
    return T


def bias(W, H, x0, y0, dither):
    if not dither:
        return np.full((H, W), 8192, np.int64)
    ys = (np.arange(y0, y0 + H) & 3)[:, None]
    xs = (np.arange(x0, x0 + W) & 3)[None, :]
    return BAYER[ys, xs]


def ref_grad(init, W, H, x0, y0, typ, rep, flags, cola, colb, pts, mode):
    T = ref_T(W, H, x0, y0, typ, rep, *pts)
    if mode == 3:
        return T.astype(np.uint32)
    b = bias(W, H, x0, y0, flags & GF_DITHER)
    A = split(np.array([[cola]], np.uint32))[0, 0]
    B = split(np.array([[colb]], np.uint32))[0, 0]
    if mode == 2:
        return ((A[3] * (16384 - T) + B[3] * T + b) >> 14).astype(np.uint8)
    if flags & GF_ALPHA_MODE:
        ramp = (A[3] * (16384 - T) + B[3] * T + b) >> 14
        c = split(init).copy()
        c[..., 3] = ((c[..., 3] * ramp + 128) * 257) >> 16
        return join(c)
    c = (A[None, None, :] * (16384 - T)[..., None] + B[None, None, :] * T[..., None] + b[..., None]) >> 14
    return join(c)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/g2")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    # accuracy of the atan2 approximation (the assembly is bit-identical to atan2a)
    rng = np.random.default_rng(5)
    yy, xx = rng.normal(size=200000), rng.normal(size=200000)
    err = np.abs(atan2a(yy, xx) - np.arctan2(yy, xx)).max()
    print(f"atan2 approximation: max error {err:.2e} rad = {err / (2 * math.pi) * 1024:.4f} / 1024 turn")
    fails = 0 if err < 2 * math.pi / 1024 else 1
    out = (ROOT / a.out).resolve()
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        fails += check_target(t, out, a)
    print("gradient_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


def grad_line(R, init, outp, W, H, x0, y0, typ, rep, flags, cola, colb, pts, reps, mode):
    R.add("grad", R.rel(init) if init else "-", R.rel(outp), W, H, x0, y0, typ, rep, flags,
          "%08X" % cola, "%08X" % colb, *[hx(v) for v in pts], reps, mode)


def check_target(target, out, a):
    rng = np.random.default_rng(7)
    R = Runner(target, out, "grcheck")
    cases = []
    geoms = [((10.0, 5.0), (90.0, 5.0)), ((3.3, 70.1), (61.7, 12.4)), ((50.5, 30.5), (58.0, 36.25)),
             ((-40.0, -10.0), (20.0, 100.0)), ((0.25, 0.75), (0.5, 0.75))]
    W, H = 83, 37
    init = rng.integers(0, 2 ** 32, (H, W), dtype=np.uint64).astype(np.uint32)
    initp = R.path("init.raw")
    write_raw(initp, init, FMT_ARGB)
    k = 0
    for gi, (s, e) in enumerate(geoms):
        pts = (s[0], s[1], e[0], e[1])
        for typ in range(7):
            for rep in range(3):
                for flags, mode in ((0, 0), (GF_DITHER, 0), (GF_ALPHA_MODE, 0), (0, 2),
                                    (GF_DITHER, 2), (0, 3)):
                    if gi >= 3 and (flags or mode == 2):
                        continue
                    cola = int(rng.integers(0, 2 ** 32))
                    colb = int(rng.integers(0, 2 ** 32))
                    x0, y0 = [(0, 0), (-7, 3), (1000, -500)][k % 3]
                    k += 1
                    p = R.path(f"g_{gi}_{typ}_{rep}_{flags}_{mode}.raw")
                    ini = initp if flags & GF_ALPHA_MODE else None
                    grad_line(R, ini, p, W, H, x0, y0, typ, rep, flags, cola, colb, pts, 1, mode)
                    exp = ref_grad(init, W, H, x0, y0, typ, rep, flags, cola, colb, pts, mode)
                    cases.append((f"{TYPES[typ]} {REPEATS[rep]} flags {flags} mode {mode} geom {gi}",
                                  exp, p, 0 if mode == 3 else 1))
    # degenerate, widths 1 / 64 / 65 / 129
    for typ in range(7):
        p = R.path(f"deg_{typ}.raw")
        pts = (5.0, 5.0, 5.0, 5.0)
        grad_line(R, None, p, 9, 3, 0, 0, typ, 0, 0, 0x11223344, 0xFF8040C0, pts, 1, 0)
        cases.append((f"degenerate {TYPES[typ]}", ref_grad(None, 9, 3, 0, 0, typ, 0, 0, 0x11223344,
                                                           0xFF8040C0, pts, 0), p, 0))
    for w in (1, 2, 63, 64, 65, 129):
        for typ in (0, 3, 5):
            pts = (0.0, 0.0, 40.0, 17.0)
            p = R.path(f"w_{w}_{typ}.raw")
            grad_line(R, None, p, w, 2, 3, 1, typ, 2, 0, 0x80FF0000, 0xFF0000FF, pts, 1, 0)
            cases.append((f"width {w} {TYPES[typ]}", ref_grad(None, w, 2, 3, 1, typ, 2, 0, 0x80FF0000,
                                                              0xFF0000FF, pts, 0), p, 0))
    res = R.run()
    fails = 0
    worst = 0
    for r, (info, exp, p, tol) in zip(res, cases):
        if not r.startswith("ok ") or not Path(p).exists():
            print(f"[{target}] FAIL {info}: {r}")
            fails += 1
            continue
        got, fmt = read_raw(p)
        if got.dtype == np.uint8:
            d = np.abs(got.astype(np.int64) - exp.astype(np.int64))
        elif exp.dtype == np.uint32 and "mode 3" in info:
            d = np.abs(got.astype(np.int64) - exp.astype(np.int64))
        else:
            d = np.abs(split(got) - split(exp))
        m = int(d.max())
        worst = max(worst, m)
        if m > tol:
            yx = np.unravel_index(np.argmax(d.reshape(d.shape[0], d.shape[1], -1).max(-1)), d.shape[:2])
            print(f"[{target}] FAIL {info}: max |d| = {m} at {yx}: got {got[yx]:#x} exp {exp[yx]:#x}")
            fails += 1
    print(f"[{target}] {len(cases)} cases, {fails} failed, max |d| = {worst}")
    if a.bench:
        names = []
        for typ in range(7):
            grad_line(R, None, R.path("bench.raw"), 4000, 3000, 0, 0, typ, 1, 0, 0xFF102030, 0x80F0E0D0,
                      (100.0, 200.0, 1900.0, 1300.0), 3, 0)
            names.append(TYPES[typ])
        grad_line(R, None, R.path("bench.raw"), 4000, 3000, 0, 0, 0, 0, GF_DITHER, 0xFF102030,
                  0x80F0E0D0, (100.0, 200.0, 1900.0, 1300.0), 3, 0)
        names.append("linear + dither")
        grad_line(R, None, R.path("bench.raw"), 4000, 3000, 0, 0, 3, 0, 0, 0xFF102030, 0x80F0E0D0,
                  (100.0, 200.0, 1900.0, 1300.0), 3, 2)
        names.append("radial, A8 (transparency mode)")
        for n, r in zip(names, R.run()):
            us = us_of(r)
            print(f"[{target}] bench gradient {n}: " +
                  (f"{us / 1000:.1f} ms = {12e6 / us:.0f} Mpx/s" if us else r))
    if not a.keep:
        shutil.rmtree(R.work, ignore_errors=True)
    return fails


if __name__ == "__main__":
    sys.exit(main())
