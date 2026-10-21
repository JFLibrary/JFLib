#!/usr/bin/env python3
"""xform_check.py -- check img/xform.s (rs_affine, xf_* matrix helpers) against numpy references.

  python tools/xform_check.py [--target win|linux|both] [--out build/g2] [--bench] [--keep]

The reference replicates the assembly exactly (f64 coordinate arithmetic in the same order,
24.8 fixed-point phases, exact copies at phase 0, working values / output conversion of
tools/resample_check.py, the same Catmull-Rom phase table), so the expected difference is
0; the check allows |d| <= 1 (<= 2 with RS_GAMMA).  Also checked: identity and integer
translations are exact (NEAREST, BILINEAR, BICUBIC) for every edge mode, a 90 degree
rotation built with xf_translate / xf_rotate / xf_invert equals img_rotate90 (NEAREST and
BILINEAR), clip rectangles leave the rest of dst untouched, thread bands are
deterministic, NaN / huge matrices do not crash, and the matrix helpers and xf_sincos are
bit-identical to the Python versions (sin / cos also within 2e-16 of math.sin / cos).
"""

import argparse
import math
import shutil
import struct
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import resample_check as rc  # noqa: E402
from resample_check import (FMT_ARGB, FMT_PARGB, RS_GAMMA, RS_NEAREST, RS_BILINEAR,  # noqa: E402
                            RS_BICUBIC, KID_CUB5, split, join, to_work, from_work, convert8,
                            write_raw, read_raw, rand_img, Runner, us_of, kern)

ROOT = Path(__file__).resolve().parent.parent
RS_MULTISAMPLE = 8
XF_TRANSPARENT, XF_CLAMP, XF_WRAP, XF_REFLECT = range(4)
EDGE_NAMES = ["transparent", "clamp", "wrap", "reflect"]
FNAME = {RS_NEAREST: "nearest", RS_BILINEAR: "bilinear", RS_BICUBIC: "bicubic",
         RS_MULTISAMPLE: "multisample"}


def hx(v):
    return "%016X" % struct.unpack("<Q", struct.pack("<d", float(v)))[0]


# ---------------------------------------------------------------- matrix helpers (bit exact)
TWO_OVER_PI = 2 / math.pi
PIO2_HI = 1.5707963267341256
PIO2_LO = 6.077100506506192e-11
S = [-1.0 / 6.0, 1.0 / 120.0, -1.0 / 5040.0, 1.0 / 362880.0, -1.0 / 39916800.0,
     1.0 / 6227020800.0, -1.0 / 1307674368000.0]
C = [-0.5, 1 / 24, -1 / 720, 1 / 40320, -1 / 3628800, 1 / 479001600, -1 / 87178291200,
     1 / 20922789888000]


def sincos(x):
    k = round(x * TWO_OVER_PI)
    kd = float(k)
    r = (x - kd * PIO2_HI) - kd * PIO2_LO
    if abs(r) < 1e-12:
        r = 0.0
    z = r * r
    p = S[6]
    for c in reversed(S[:6]):
        p = p * z + c
    s = (r * z) * p + r
    p = C[7]
    for c in reversed(C[:7]):
        p = p * z + c
    co = p * z + 1.0
    q = k & 3
    return [(s, co), (co, -s), (-s, -co), (-co, s)][q]


def mat_mul(A, B):
    return [A[0] * B[0] + A[1] * B[3], A[0] * B[1] + A[1] * B[4],
            (A[0] * B[2] + A[1] * B[5]) + A[2],
            A[3] * B[0] + A[4] * B[3], A[3] * B[1] + A[4] * B[4],
            (A[3] * B[2] + A[4] * B[5]) + A[5]]


def translate(m, tx, ty):
    m = list(m)
    m[2] = (m[0] * tx + m[1] * ty) + m[2]
    m[5] = (m[3] * tx + m[4] * ty) + m[5]
    return m


def scale(m, sx, sy):
    return [m[0] * sx, m[1] * sy, m[2], m[3] * sx, m[4] * sy, m[5]]


def rotate(m, ang, cx, cy):
    m = translate(m, cx, cy)
    s, c = sincos(ang)
    m = mat_mul(m, [c, -s, 0.0, s, c, 0.0])
    return translate(m, -cx, -cy)


def invert(m):
    a, b, c, d, e, f = m
    det = a * e - b * d
    if not math.isfinite(det) or det == 0.0:
        return None
    inv = 1.0 / det
    a2, b2, d2, e2 = e * inv, (-b) * inv, (-d) * inv, a * inv
    return [a2, b2, -(a2 * c + b2 * f), d2, e2, -(d2 * c + e2 * f)]


def apply(m, x, y):
    return (m[0] * x + m[1] * y) + m[2], (m[3] * x + m[4] * y) + m[5]


# ---------------------------------------------------------------- sampling reference
def cubic_lut():
    out = []
    for p in range(256):
        f = p / 256.0
        w = [kern(KID_CUB5, 1.0 + f), kern(KID_CUB5, f), kern(KID_CUB5, 1.0 - f),
             kern(KID_CUB5, 2.0 - f)]
        tot = 0.0
        for x in w:
            tot += x
        q = [min(max(math.floor(x / tot * 16384.0 + 0.5), -32767), 32767) for x in w]
        am = max(range(4), key=lambda k: (q[k], -k))
        q[am] += 16384 - sum(q)
        out.append(q)
    return np.array(out, np.int64)


LUT = cubic_lut()
LO, HI = -2.0 ** 30, 2.0 ** 30
MSOFF = [0.125, 0.375, 0.625, 0.875]


def clampnan(t):
    return np.where(np.isnan(t), LO, np.clip(t, LO, HI))


def emap(j, n, mode):
    if mode == XF_TRANSPARENT:
        return np.where((j >= 0) & (j < n), j, -1)
    if mode == XF_CLAMP:
        return np.clip(j, 0, n - 1)
    if mode == XF_WRAP:
        return np.mod(j, n)
    m = np.mod(j, 2 * n)
    return np.where(m < n, m, 2 * n - 1 - m)


def fetch(img, X, Y, mode):
    h, w = img.shape
    xi = emap(X, w, mode)
    yi = emap(Y, h, mode)
    ok = (xi >= 0) & (yi >= 0)
    v = img[np.where(ok, yi, 0), np.where(ok, xi, 0)]
    return np.where(ok, v, 0).astype(np.uint32)


def work(raw, fmt, gamma):
    sh = raw.shape
    return to_work(raw.reshape(-1, 1), fmt, gamma).reshape(sh + (4,))


def uv(qx, qy):
    tx = clampnan((qx - 0.5) * 256.0 + 0.5)
    ty = clampnan((qy - 0.5) * 256.0 + 0.5)
    return np.floor(tx).astype(np.int64), np.floor(ty).astype(np.int64)


def bil(img, sfmt, gamma, U, V, mode):
    x0, y0 = U >> 8, V >> 8
    wx = ((U & 255) * 64)[..., None]
    wy = ((V & 255) * 64)[..., None]
    P = [work(fetch(img, x0 + dx, y0 + dy, mode), sfmt, gamma) for dy in (0, 1) for dx in (0, 1)]
    T = rc.sat16((P[0] * (16384 - wx) + P[1] * wx + 8192) >> 14)
    B = rc.sat16((P[2] * (16384 - wx) + P[3] * wx + 8192) >> 14)
    return (T * (16384 - wy) + B * wy + 8192) >> 14


def ref_affine(img, sfmt, W, H, dfmt, filt, flags, m, clip=None, fill=0):
    a, b, c, d, e, f = m
    mode = (flags >> 8) & 3
    gamma = bool(flags & RS_GAMMA)
    out = np.full((H, W), fill, np.uint32)
    cx0, cy0, cx1, cy1 = (0, 0, W, H) if clip is None else clip
    cx0, cy0, cx1, cy1 = max(cx0, 0), max(cy0, 0), min(cx1, W), min(cy1, H)
    if cx0 >= cx1 or cy0 >= cy1:
        return out
    ys = np.arange(cy0, cy1, dtype=np.float64)
    xs = np.arange(cx0, cx1, dtype=np.float64)
    xc = (xs + 0.5)[None, :]
    yc = (ys + 0.5)[:, None]
    qx = a * xc + (b * yc + c)
    qy = d * xc + (e * yc + f)
    if filt == RS_NEAREST:
        sx = np.floor(clampnan(qx)).astype(np.int64)
        sy = np.floor(clampnan(qy)).astype(np.int64)
        res = convert8(fetch(img, sx, sy, mode), sfmt, dfmt)
        out[cy0:cy1, cx0:cx1] = res
        return out
    if filt in (RS_BILINEAR, RS_BICUBIC):
        U, V = uv(qx, qy)
        exact = ((U | V) & 255) == 0
        x0, y0 = U >> 8, V >> 8
        if filt == RS_BILINEAR:
            P = bil(img, sfmt, gamma, U, V, mode)
        else:
            wx = LUT[U & 255]
            wy = LUT[V & 255]
            R = []
            for dy in range(4):
                acc = 0
                for dx in range(4):
                    Pt = work(fetch(img, x0 - 1 + dx, y0 - 1 + dy, mode), sfmt, gamma)
                    acc = acc + wx[..., dx:dx + 1] * Pt
                R.append(rc.sat16((acc + 8192) >> 14))
            acc = 0
            for dy in range(4):
                acc = acc + wy[..., dy:dy + 1] * R[dy]
            P = rc.sat16((acc + 8192) >> 14)
        res = from_work(P, dfmt, gamma)
        raw = convert8(fetch(img, x0, y0, mode), sfmt, dfmt)
        out[cy0:cy1, cx0:cx1] = np.where(exact, raw, res)
        return out
    # multisample
    acc = 0
    for t in range(4):
        yct = (ys + MSOFF[t])[:, None]
        rowx = b * yct + c
        rowy = e * yct + f
        for s in range(4):
            xcs = (xs + MSOFF[s])[None, :]
            U, V = uv(a * xcs + rowx, d * xcs + rowy)
            acc = acc + bil(img, sfmt, gamma, U, V, mode)
    P = rc.sat16((acc + 8) >> 4)
    out[cy0:cy1, cx0:cx1] = from_work(P, dfmt, gamma)
    return out


# ---------------------------------------------------------------- tests
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/g2")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    fails = 0
    for t in targets:
        fails += check_target(t, out, a)
    print("xform_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


def affine_line(R, src, dst, W, H, dfmt, filt, flags, m, clip=None, fill=0, reps=1, split=1):
    cl = (-1, 0, 0, 0) if clip is None else clip
    R.add("affine", R.rel(src), R.rel(dst), W, H, dfmt, filt, flags, "%08X" % fill, reps, split,
          *cl, *[hx(v) for v in m])


def check_target(target, out, a):
    rng = np.random.default_rng(99)
    R = Runner(target, out, "xfcheck")
    cases = []

    # --- matrix helpers and sin / cos
    angles = [0.0, math.pi / 2, math.pi, -math.pi / 2, 3 * math.pi / 2, 2 * math.pi, 0.3, -0.7,
              1e-13, 12.5, -100.25, 1234.5678, math.pi / 4, 5 * math.pi / 2, 1e5 + 0.1]
    p = R.path("sincos.bin")
    R.add("sincos", R.rel(p), *[hx(v) for v in angles])
    cases.append(("sincos", angles, p))
    seqs = [
        [("T", 3.5, -2.25), ("R", 0.3, 10.0, 20.0), ("S", 1.5, 0.75), ("P", 1.0, 2.0), ("I",),
         ("P", 1.0, 2.0)],
        [("R", math.pi / 2, 0.0, 0.0), ("T", 7.0, 0.0), ("I",), ("P", 0.5, 0.5)],
        [("M", 1.0, 0.25, 3.0, -0.5, 2.0, 1.0), ("R", -1.1, 3.0, 4.0), ("I",), ("I",), ("P", -3.0, 9.0)],
    ]
    for k, seq in enumerate(seqs):
        p = R.path(f"xfmat{k}.bin")
        toks = []
        for op in seq:
            toks.append(op[0])
            toks += [hx(v) for v in op[1:]]
        R.add("xfmat", R.rel(p), *toks)
        cases.append(("xfmat", seq, p))
    p = R.path("lut.bin")
    R.add("cubiclut", R.rel(p))
    cases.append(("lut", None, p))

    # --- identity and integer translations: exact
    for sfmt in (FMT_ARGB, FMT_PARGB):
        img = rand_img(rng, 19, 13, sfmt, 0.3)
        src = R.path(f"id_{sfmt}.raw")
        write_raw(src, img, sfmt)
        for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC):
            for flags in (0, RS_GAMMA):
                p = R.path(f"id_{sfmt}_{filt}_{flags}.raw")
                affine_line(R, src, p, 19, 13, sfmt, filt, flags, [1, 0, 0, 0, 1, 0])
                cases.append(("exact", f"identity {FNAME[filt]} s{sfmt} g{flags}", img, p, 0))
            for edge in range(4):
                for (tx, ty) in ((3, -2), (-5, 4), (25, 0)):
                    m = [1, 0, -tx, 0, 1, -ty]                  # dst(x) = src(x - tx)
                    fl = edge << 8
                    exp = shift_ref(img, tx, ty, edge)
                    p = R.path(f"tr_{sfmt}_{filt}_{edge}_{tx}.raw")
                    affine_line(R, src, p, 19, 13, sfmt, filt, fl, m)
                    cases.append(("exact", f"translate {tx},{ty} {FNAME[filt]} {EDGE_NAMES[edge]} s{sfmt}",
                                  exp, p, 0))

    # --- rotate by 90 degrees == img_rotate90 (nearest and bilinear)
    for sfmt in (FMT_ARGB, FMT_PARGB):
        img = rand_img(rng, 23, 11, sfmt, 0.3)
        src = R.path(f"r90_{sfmt}.raw")
        write_raw(src, img, sfmt)
        h, w = img.shape
        for d, ang, tx, ty in ((1, math.pi / 2, h, 0), (-1, -math.pi / 2, 0, w)):
            pr = R.path(f"r90_ref_{sfmt}_{d}.raw")
            R.add("rot90", R.rel(src), R.rel(pr), d)
            for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC):
                p = R.path(f"r90_{sfmt}_{d}_{filt}.raw")
                R.add("xfrot", R.rel(src), R.rel(p), h, w, sfmt, filt, 0, hx(ang), hx(tx), hx(ty))
                cases.append(("rot90", f"rotate90 dir {d} {FNAME[filt]} s{sfmt}", pr, p, 0))

    # --- general transforms vs the reference
    mats = {
        "rot17": invert(rotate(translate([1, 0, 0, 0, 1, 0], 5.0, -3.0), 0.2967, 20.0, 15.0)),
        "rot-33s": invert(scale(rotate([1, 0, 0, 0, 1, 0], -0.576, 12.0, 9.0), 1.3, 0.8)),
        "down": [2.7, 0.0, 0.3, 0.0, 2.3, -0.4],
        "up": [0.37, 0.05, 1.2, -0.02, 0.41, 0.7],
        "skew": [1.0, 0.45, -3.0, 0.2, 1.0, 1.0],
        "rot180": invert(rotate([1, 0, 0, 0, 1, 0], math.pi, 20.0, 15.0)),
    }
    for sfmt in (FMT_ARGB, FMT_PARGB):
        img = rand_img(rng, 40, 30, sfmt, 0.15)
        src = R.path(f"g_{sfmt}.raw")
        write_raw(src, img, sfmt)
        for mi, (mn, m) in enumerate(mats.items()):
            for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE):
                for edge in range(4):
                    for dfmt in (FMT_ARGB, FMT_PARGB):
                        for flags in ((0, RS_GAMMA) if filt != RS_NEAREST else (0,)):
                            if (edge + mi + dfmt) % 2 and flags:
                                continue                        # keep the count down
                            fl = flags | edge << 8
                            W, H = 37, 29
                            p = R.path(f"g_{sfmt}_{mi}_{filt}_{edge}_{dfmt}_{flags}.raw")
                            affine_line(R, src, p, W, H, dfmt, filt, fl, m)
                            exp = ref_affine(img, sfmt, W, H, dfmt, filt, fl, m)
                            cases.append(("affine", f"{mn} {FNAME[filt]} {EDGE_NAMES[edge]} s{sfmt} "
                                          f"d{dfmt} g{flags}", exp, p, 2 if flags else 1))
    # clip rectangle, fill outside, thread bands
    img = rand_img(rng, 64, 48, FMT_ARGB, 0.1)
    src = R.path("clip.raw")
    write_raw(src, img, FMT_ARGB)
    m = mats["rot17"]
    for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE):
        for clip in ((5, 7, 40, 33), (-10, -10, 200, 3), (60, 40, 70, 90), (10, 10, 10, 20)):
            p = R.path(f"clip_{filt}_{clip[0]}_{clip[1]}.raw")
            affine_line(R, src, p, 64, 48, FMT_ARGB, filt, 0, m, clip, 0x12345678)
            exp = ref_affine(img, FMT_ARGB, 64, 48, FMT_ARGB, filt, 0, m, clip, 0x12345678)
            cases.append(("affine", f"clip {clip} {FNAME[filt]}", exp, p, 1))
        for split in (1, 3, 7):
            p = R.path(f"split_{filt}_{split}.raw")
            affine_line(R, src, p, 71, 53, FMT_PARGB, filt, XF_REFLECT << 8, m, None, 0, 1, split)
            exp = ref_affine(img, FMT_ARGB, 71, 53, FMT_PARGB, filt, XF_REFLECT << 8, m)
            cases.append(("affine", f"split {split} {FNAME[filt]}", exp, p, 1))
    # tiny sources (1 x 1, 1 x N, N x 1): every filter and edge mode
    for (w, h) in ((1, 1), (1, 5), (6, 1)):
        tiny = rand_img(rng, w, h, FMT_ARGB, 0.2)
        tsrc = R.path(f"tiny_{w}x{h}.raw")
        write_raw(tsrc, tiny, FMT_ARGB)
        m = invert(rotate(scale([1, 0, 0, 0, 1, 0], 0.4, 0.6), 0.5, 1.0, 1.0))
        for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE):
            for edge in range(4):
                p = R.path(f"tiny_{w}x{h}_{filt}_{edge}.raw")
                affine_line(R, tsrc, p, 7, 5, FMT_PARGB, filt, edge << 8, m)
                exp = ref_affine(tiny, FMT_ARGB, 7, 5, FMT_PARGB, filt, edge << 8, m)
                cases.append(("affine", f"tiny {w}x{h} {FNAME[filt]} {EDGE_NAMES[edge]}", exp, p, 1))
    # row offsets beyond 4 GB (1 MB + 4 byte strides): vertical flips
    tall = rand_img(rng, 3, 4200, FMT_ARGB, 0.05)
    bsrc = R.path("bigstride.raw")
    write_raw(bsrc, tall, FMT_ARGB)
    R.add("stride", (1 << 20) + 4)
    cases.append(("none",))
    for filt, f in ((RS_NEAREST, 4200.0), (RS_BILINEAR, 4200.3), (RS_BICUBIC, 4199.8)):
        m = [1, 0, 0.25, 0, -1, f]
        p = R.path(f"bigstride_{filt}.raw")
        affine_line(R, bsrc, p, 3, 4200, FMT_PARGB, filt, XF_CLAMP << 8, m)
        exp = ref_affine(tall, FMT_ARGB, 3, 4200, FMT_PARGB, filt, XF_CLAMP << 8, m)
        cases.append(("affine", f"stride > 4 GB flip {FNAME[filt]}", exp, p, 1))
    R.add("stride", 0)
    cases.append(("none",))
    # degenerate matrices: NaN, huge, zero (all map far outside or onto one point)
    for mn, m in (("nan", [float("nan"), 0, 0, 0, 1, 0]), ("huge", [1e300, 0, -1e300, 0, 1e300, 5]),
                  ("zero", [0, 0, 3.3, 0, 0, 4.7]), ("inf", [float("inf"), 0, 0, 0, 1, 0])):
        for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE):
            for edge in (XF_TRANSPARENT, XF_WRAP, XF_REFLECT):
                p = R.path(f"deg_{mn}_{filt}_{edge}.raw")
                affine_line(R, src, p, 9, 7, FMT_ARGB, filt, edge << 8, m)
                exp = ref_affine(img, FMT_ARGB, 9, 7, FMT_ARGB, filt, edge << 8, m)
                cases.append(("affine", f"degenerate {mn} {FNAME[filt]} {EDGE_NAMES[edge]}", exp, p, 1))

    res = R.run()
    fails = evaluate(target, res, cases)
    if a.bench:
        bench(R, rng)
    if not a.keep:
        shutil.rmtree(R.work, ignore_errors=True)
    return fails


def shift_ref(img, tx, ty, edge):
    h, w = img.shape
    X = np.arange(w)[None, :] - tx
    Y = np.arange(h)[:, None] - ty
    return fetch(img, np.broadcast_to(X, (h, w)), np.broadcast_to(Y, (h, w)), edge)


def evaluate(target, results, cases):
    fails = 0
    worst = {}
    for res, case in zip(results, cases):
        kind = case[0]
        if kind == "none" and res.startswith("ok "):
            continue
        if not res.startswith("ok "):
            print(f"[{target}] FAIL {kind} {case[1]}: {res}")
            fails += 1
            continue
        if kind == "sincos":
            angles, p = case[1], case[2]
            got = np.frombuffer(Path(p).read_bytes(), np.float64).reshape(-1, 2)
            for ang, (gs, gc) in zip(angles, got):
                es, ec = sincos(ang)
                # remainders below 1e-12 are snapped to 0 on purpose (exact quarter turns)
                tol = 1.01e-12 if abs(ang - round(ang * TWO_OVER_PI) * (math.pi / 2)) < 2e-12 else 0.0
                tol = max(tol, 2e-16 * max(1.0, abs(ang)))
                bad = (struct.pack("<dd", gs, gc) != struct.pack("<dd", es, ec)
                       or abs(gs - math.sin(ang)) > tol or abs(gc - math.cos(ang)) > tol)
                if bad:
                    print(f"[{target}] FAIL sincos({ang!r}): got {gs!r}, {gc!r} exp {es!r}, {ec!r} "
                          f"math {math.sin(ang)!r}, {math.cos(ang)!r}")
                    fails += 1
            continue
        if kind == "xfmat":
            seq, p = case[1], case[2]
            m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            pts = []
            for op in seq:
                if op[0] == "T":
                    m = translate(m, op[1], op[2])
                elif op[0] == "S":
                    m = scale(m, op[1], op[2])
                elif op[0] == "R":
                    m = rotate(m, op[1], op[2], op[3])
                elif op[0] == "I":
                    m = invert(m)
                elif op[0] == "M":
                    m = mat_mul(m, list(op[1:]))
                elif op[0] == "P":
                    pts += list(apply(m, op[1], op[2]))
            got = np.frombuffer(Path(p).read_bytes(), np.float64)
            exp = np.array(m + pts, np.float64)
            if got.tobytes() != exp.tobytes():
                print(f"[{target}] FAIL xfmat {seq}: got {got.tolist()} exp {exp.tolist()}")
                fails += 1
            continue
        if kind == "lut":
            got = np.frombuffer(Path(case[2]).read_bytes(), np.int16).reshape(256, 4)
            if not (got == LUT).all():
                print(f"[{target}] FAIL cubic lut")
                fails += 1
            continue
        info, exp, p, tol = case[1], case[2], case[3], case[4]
        if kind == "rot90":
            exp, _ = read_raw(exp)
        if not Path(p).exists():
            print(f"[{target}] FAIL {kind} {info}: no output")
            fails += 1
            continue
        got, _ = read_raw(p)
        if got.shape != exp.shape:
            print(f"[{target}] FAIL {kind} {info}: shape {got.shape} != {exp.shape}")
            fails += 1
            continue
        d = np.abs(split(got) - split(exp))
        mx = int(d.max()) if d.size else 0
        worst[kind] = max(worst.get(kind, 0), mx)
        if mx > tol:
            yx = np.unravel_index(np.argmax(d.max(-1)), d.shape[:2])
            print(f"[{target}] FAIL {kind} {info}: max |d| = {mx} at {yx}: got "
                  f"{split(got)[yx].tolist()} exp {split(exp)[yx].tolist()}")
            fails += 1
    print(f"[{target}] {len(cases)} cases, {fails} failed; max |d| per kind: {worst}")
    return fails


def bench(R, rng):
    img = rand_img(rng, 4000, 3000, FMT_ARGB, 0.02)
    src = R.path("bench.raw")
    write_raw(src, img, FMT_ARGB)
    m = invert(rotate([1, 0, 0, 0, 1, 0], math.radians(30), 2000.0, 1500.0))
    names = []
    for filt in (RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE):
        affine_line(R, src, R.path("bo.raw"), 4000, 3000, FMT_ARGB, filt, 0, m, reps=1)
        names.append(f"rotate 30 deg 4000x3000 {FNAME[filt]}")
    affine_line(R, src, R.path("bo.raw"), 4000, 3000, FMT_ARGB, RS_BILINEAR, 0, [1, 0, -7, 0, 1, 3])
    names.append("translate 4000x3000 bilinear (exact copies)")
    affine_line(R, src, R.path("bo.raw"), 4000, 3000, FMT_ARGB, RS_BICUBIC, 0, m, split=8)
    names.append("rotate 30 deg 4000x3000 bicubic, 8 bands on the pool")
    res = R.run()
    for n, r in zip(names, res):
        us = us_of(r)
        print(f"[{R.target}] bench {n}: " + (f"{us / 1000:.1f} ms" if us is not None else r))


if __name__ == "__main__":
    sys.exit(main())
