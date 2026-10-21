#!/usr/bin/env python3
"""dash_check.py -- check gfx/dash.s (dash_stroke, dash_pattern) against a geometric Python
reference (agent T4).

  python tools/dash_check.py [--target win|linux|both] [--out build/t4] [--keep] [--bench]

Every case strokes a path (pathdata.s, polylines and one curve) with rstest's `dash` op and
compares the mask with the union of the reference primitives, sampled 16 x 16 per pixel:
segment rectangles (square caps extend them by w / 2), discs for round joins / caps / dots,
mitre wedges / bevel triangles, arrow heads (filled triangle, open = two legs stroked with
the line's width) -- the dash pieces follow the same walk as the assembly (phase, zero-length
entries become dots, the path's own start / end keep DSS_CAP0 / DSS_CAP1, inner ends
DSS_DCAP, a closed subpath's pattern runs once around).
Tolerance: round arcs are polygons with a chord error <= 0.05 px and the supersampling
error is ~1/16 per edge pixel, so |d| <= 40 / 255 per pixel, 99.5 % of the pixels within
20, mean |d| <= 0.6.  Cases whose pieces overlap (arrow legs over the line, square dash caps
at a corner, the curve) allow 72 at single pixels: raster.s accumulates signed coverage per
pixel, so two partial coverages of overlapping contours add up (min(sum, 1)) instead of
forming the exact union.
"""

import argparse
import math
import shutil
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from resample_check import read_raw, Runner  # noqa: E402
from pathdata_check import parse  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SS = 16
FLAT, ROUND, SQUARE, ARROW, ARROWF = 0, 1, 2, 3, 4
J_MITER, J_ROUND, J_BEVEL = 0, 1, 2
PATS = {"-": [], "d1": [3, 1], "d2": [1, 1], "d3": [3, 1, 1, 1], "d4": [3, 1, 1, 1, 1, 1]}


class Canvas:
    def __init__(self, W, H, ss=SS):
        self.W, self.H, self.ss = W, H, ss
        self.m = np.zeros((H * ss, W * ss), bool)

    def _box(self, x0, y0, x1, y1):
        s = self.ss
        c0 = max(0, int(math.floor(x0 * s - 1)))
        r0 = max(0, int(math.floor(y0 * s - 1)))
        c1 = min(self.W * s, int(math.ceil(x1 * s + 1)))
        r1 = min(self.H * s, int(math.ceil(y1 * s + 1)))
        if c1 <= c0 or r1 <= r0:
            return None
        X = (np.arange(c0, c1) + 0.5) / s
        Y = (np.arange(r0, r1) + 0.5) / s
        return (slice(r0, r1), slice(c0, c1)), X[None, :], Y[:, None]

    def rect(self, a, b, hw, e0=0.0, e1=0.0):
        L = math.hypot(b[0] - a[0], b[1] - a[1])
        if L == 0:
            return
        ux, uy = (b[0] - a[0]) / L, (b[1] - a[1]) / L
        r = hw + max(e0, e1) + 1
        bx = self._box(min(a[0], b[0]) - r, min(a[1], b[1]) - r, max(a[0], b[0]) + r,
                       max(a[1], b[1]) + r)
        if bx is None:
            return
        sl, X, Y = bx
        dx, dy = X - a[0], Y - a[1]
        t = dx * ux + dy * uy
        n = np.abs(dx * uy - dy * ux)
        self.m[sl] |= (n <= hw) & (t >= -e0) & (t <= L + e1)

    def disc(self, c, r):
        bx = self._box(c[0] - r, c[1] - r, c[0] + r, c[1] + r)
        if bx is None:
            return
        sl, X, Y = bx
        self.m[sl] |= (X - c[0]) ** 2 + (Y - c[1]) ** 2 <= r * r

    def poly(self, pts):
        xs = [p[0] for p in pts]
        ys = [p[1] for p in pts]
        bx = self._box(min(xs), min(ys), max(xs), max(ys))
        if bx is None:
            return
        sl, X, Y = bx
        w = np.zeros(np.broadcast(X, Y).shape, int)
        for i in range(len(pts)):
            (x0, y0), (x1, y1) = pts[i], pts[(i + 1) % len(pts)]
            if y0 == y1:
                continue
            up = (y0 <= Y) & (Y < y1)
            dn = (y1 <= Y) & (Y < y0)
            xi = x0 + (Y - y0) * (x1 - x0) / (y1 - y0)
            w = w + np.where(up & (X < xi), 1, 0) - np.where(dn & (X < xi), 1, 0)
        self.m[sl] |= w != 0

    def coverage(self):
        s = self.ss
        return self.m.reshape(self.H, s, self.W, s).mean(axis=(1, 3))


def unit(a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    L = math.hypot(dx, dy)
    return (dx / L, dy / L) if L > 1e-12 else (1.0, 0.0)


def join(cv, P, din, dout, hw, jn, mlim=4.0):
    if jn == J_ROUND:
        cv.disc(P, hw)
        return
    cross = din[0] * dout[1] - din[1] * dout[0]
    dot = din[0] * dout[0] + din[1] * dout[1]
    if abs(cross) <= 1e-9 and dot > 0:
        return
    s = 1 if cross > 0 else -1          # the outer side is +n for cross > 0
    nin = (din[1] * s, -din[0] * s)
    nout = (dout[1] * s, -dout[0] * s)
    a = (P[0] + hw * nin[0], P[1] + hw * nin[1])
    b = (P[0] + hw * nout[0], P[1] + hw * nout[1])
    if jn == J_MITER and 1 + dot > 1e-9 and math.sqrt(2 / (1 + dot)) <= mlim:
        f = hw / (1 + dot)
        m = (P[0] + f * (nin[0] + nout[0]), P[1] + f * (nin[1] + nout[1]))
        cv.poly([P, a, m, b])
    else:
        cv.poly([P, a, b])


def stroke_open(cv, pts, hw, c0, c1, jn):
    n = len(pts)
    for i in range(n - 1):
        e0 = hw if (i == 0 and c0 == SQUARE) else 0.0
        e1 = hw if (i == n - 2 and c1 == SQUARE) else 0.0
        cv.rect(pts[i], pts[i + 1], hw, e0, e1)
    for i in range(1, n - 1):
        join(cv, pts[i], unit(pts[i - 1], pts[i]), unit(pts[i], pts[i + 1]), hw, jn)
    if c0 == ROUND:
        cv.disc(pts[0], hw)
    if c1 == ROUND:
        cv.disc(pts[-1], hw)


def stroke_ring(cv, pts, hw, jn):
    n = len(pts)
    for i in range(n):
        cv.rect(pts[i], pts[(i + 1) % n], hw)
    for i in range(n):
        join(cv, pts[i], unit(pts[i - 1], pts[i]), unit(pts[i], pts[(i + 1) % n]), hw, jn)


def dot(cv, P, d, hw, cap):
    if cap == ROUND:
        cv.disc(P, hw)
    elif cap == SQUARE:
        n = (d[1], -d[0])
        cv.poly([(P[0] + hw * (sd * d[0] + sn * n[0]), P[1] + hw * (sd * d[1] + sn * n[1]))
                 for sd, sn in ((1, 1), (1, -1), (-1, -1), (-1, 1))])


def push_nd(lst, q):
    if not lst or (q[0] - lst[-1][0]) ** 2 + (q[1] - lst[-1][1]) ** 2 > 1e-12:
        lst.append(q)


def pieces(pts, closed, pat, phase):
    npt = len(pat)
    k, o, guard = 0, phase, 0
    while o >= pat[k]:
        guard += 1
        if guard > 2 * npt:
            break
        o -= pat[k]
        k = (k + 1) % npt
    rem = max(pat[k] - o, 0.0)
    on = k % 2 == 0
    out = []
    cur = [pts[0]] if on else []
    atstart = True
    d = (1.0, 0.0)
    segs = len(pts) if closed else len(pts) - 1
    for s in range(segs):
        a, b = pts[s], pts[(s + 1) % len(pts)]
        d = unit(a, b)
        L = math.hypot(b[0] - a[0], b[1] - a[1])
        t = 0.0
        while L - t > rem:
            t += rem
            q = (a[0] + d[0] * t, a[1] + d[1] * t)
            if on:
                push_nd(cur, q)
                out.append((cur, "c0" if atstart else "dc", "dc", d))
                cur = []
            else:
                cur = [q]
            atstart = False
            on = not on
            k = (k + 1) % npt
            rem = pat[k]
        rem -= L - t
        if on:
            push_nd(cur, b)
    if on and cur:
        out.append((cur, "c0" if atstart else "dc", "dc" if closed else "c1", d))
    return out


def trim(pts, dist, at_end):
    p = list(pts[::-1] if at_end else pts)
    i = 0
    while i + 1 < len(p):
        L = math.hypot(p[i + 1][0] - p[i][0], p[i + 1][1] - p[i][1])
        if L > dist:
            f = dist / L
            p[i] = (p[i][0] + (p[i + 1][0] - p[i][0]) * f, p[i][1] + (p[i + 1][1] - p[i][1]) * f)
            p = p[i:]
            return p[::-1] if at_end else p
        dist -= L
        i += 1
    return []


def arrow(cv, tip, d, w, filled, jn):
    L = 3 * w + 4
    H = L / 2
    n = (d[1], -d[0])
    base = (tip[0] - d[0] * L, tip[1] - d[1] * L)
    left = (base[0] + n[0] * H, base[1] + n[1] * H)
    right = (base[0] - n[0] * H, base[1] - n[1] * H)
    if filled:
        cv.poly([left, tip, right])
    else:
        stroke_open(cv, [left, tip, right], w / 2, FLAT, FLAT, jn)


def dedupe(pts, closed):
    out = []
    for q in pts:
        push_nd(out, q)
    if closed:
        while len(out) >= 2 and (out[-1][0] - out[0][0]) ** 2 + (out[-1][1] - out[0][1]) ** 2 <= 1e-12:
            out.pop()
    return out


def reference(W, H, data, w, pat, c0, c1, dcap, jn, offset):
    _rule, subs, _n = parse(data)
    cv = Canvas(W, H)
    hw = w / 2
    patpx = [max(v, 0) * w for v in pat]
    patpx = [v if v >= 1e-4 else 0.0 for v in patpx]
    if len(patpx) % 2:
        patpx = patpx * 2
    if sum(patpx) < 0.25:
        patpx = []
    phase = 0.0
    if patpx:
        per = sum(patpx)
        phase = offset * w - math.floor(offset * w / per) * per
    for pts, closed in subs:
        pts = dedupe([(float(np.float32(x)), float(np.float32(y))) for x, y in pts], closed)
        if not pts:
            continue
        if len(pts) == 1:
            dot(cv, pts[0], (1.0, 0.0), hw, c0 if c0 in (ROUND, SQUARE) else c1)
            continue
        if closed:
            bc0 = bc1 = dcap
            body = pts
        else:
            tip0, d0 = pts[0], unit(pts[1], pts[0])
            tip1, d1 = pts[-1], unit(pts[-2], pts[-1])
            body = pts
            if c0 == ARROWF:
                body = trim(body, (3 * w + 4) / 2, False)
            if c1 == ARROWF and body:
                body = trim(body, (3 * w + 4) / 2, True)
            bc0 = FLAT if c0 >= ARROW else c0
            bc1 = FLAT if c1 >= ARROW else c1
        if len(body) >= 2:
            if not patpx:
                if closed:
                    stroke_ring(cv, body, hw, jn)
                else:
                    stroke_open(cv, body, hw, bc0, bc1, jn)
            else:
                caps = {"c0": bc0, "c1": bc1, "dc": dcap}
                for pc, cs, ce, d in pieces(body, closed, patpx, phase):
                    if len(pc) == 1:
                        cap = caps[cs] if caps[cs] in (ROUND, SQUARE) else caps[ce]
                        dot(cv, pc[0], d, hw, cap)
                    else:
                        stroke_open(cv, pc, hw, caps[cs], caps[ce], jn)
        if not closed:
            if c0 >= ARROW:
                arrow(cv, tip0, d0, w, c0 == ARROWF, jn)
            if c1 >= ARROW:
                arrow(cv, tip1, d1, w, c1 == ARROWF, jn)
    return cv.coverage()


POLY = "M 12 20 L 82 20 L 60 76"
CASES = [
    # name, data, W, H, width, pattern, cap0, cap1, dcap, join, offset
    ("solid_round", POLY, 96, 96, 6, "-", ROUND, ROUND, FLAT, J_ROUND, 0),
    ("solid_flat", POLY, 96, 96, 6, "-", FLAT, FLAT, FLAT, J_ROUND, 0),
    ("solid_square", POLY, 96, 96, 7, "-", SQUARE, SQUARE, FLAT, J_ROUND, 0),
    ("solid_miter", POLY, 96, 96, 8, "-", FLAT, FLAT, FLAT, J_MITER, 0),
    ("solid_bevel", POLY, 96, 96, 8, "-", FLAT, ROUND, FLAT, J_BEVEL, 0),
    ("sharp_miterlimit", "M 10 30 L 86 40 L 10 50", 96, 80, 6, "-", FLAT, FLAT, FLAT, J_MITER, 0),
    ("dash", POLY, 96, 96, 4, "d1", FLAT, FLAT, FLAT, J_ROUND, 0),
    ("dot_round", POLY, 96, 96, 5, "d2", ROUND, ROUND, ROUND, J_ROUND, 0),
    ("dashdot_off", POLY, 96, 96, 3, "d3", FLAT, FLAT, FLAT, J_ROUND, 1.5),
    ("dashdotdot_sq", POLY, 96, 96, 3, "d4", SQUARE, SQUARE, SQUARE, J_ROUND, 0),
    ("zero_dots", POLY, 96, 96, 5, "2000,1500,0,1500", ROUND, ROUND, ROUND, J_ROUND, 0),
    ("odd_pattern", POLY, 96, 96, 3, "2000,1000,3000", FLAT, FLAT, FLAT, J_ROUND, 0),
    ("ring_miter", "M 20 20 H 76 V 76 H 20 Z", 96, 96, 8, "-", FLAT, FLAT, FLAT, J_MITER, 0),
    ("ring_ccw", "M 20 20 V 76 H 76 V 20 Z", 96, 96, 6, "-", FLAT, FLAT, FLAT, J_ROUND, 0),
    ("ring_dash", "M 20 20 H 76 V 76 H 20 Z", 96, 96, 4, "d1", FLAT, FLAT, FLAT, J_ROUND, 0),
    ("arrows", "M 14 70 L 50 30 L 84 60", 96, 96, 4, "-", ARROW, ARROWF, FLAT, J_ROUND, 0),
    ("arrows_dash", "M 10 80 L 86 16", 96, 96, 3, "d1", ARROWF, ARROW, FLAT, J_ROUND, 0),
    ("arrow_thin", "M 10 48 L 86 48", 96, 96, 1, "-", ARROWF, ARROWF, FLAT, J_ROUND, 0),
    ("two_subpaths", "M 10 10 L 86 10 M 10 30 L 86 86", 96, 96, 5, "d2", ROUND, SQUARE, FLAT,
     J_ROUND, 0),
    ("single_point", "M 40 40 Z M 60 60 L 60 60", 96, 96, 9, "-", ROUND, ROUND, FLAT, J_ROUND, 0),
    ("curve", "M 10 80 C 10 0 86 0 86 80", 96, 96, 5, "d1", ROUND, ROUND, ROUND, J_ROUND, 0),
    ("wide", "M 30 30 L 66 66", 96, 96, 40, "-", ROUND, SQUARE, FLAT, J_ROUND, 0),
]


OVERLAP = {"curve", "arrows", "arrows_dash", "arrow_thin", "dashdotdot_sq"}


def pat_of(spec):
    if spec in PATS:
        return PATS[spec]
    return [int(v) / 1000 for v in spec.split(",")]


def check_target(target, out, a):
    R = Runner(target, out, name="dashcheck")
    work = R.work
    for c in CASES:
        name, data, W, H, w, pat, c0, c1, dc, jn, off = c
        (work / (name + ".txt")).write_text(data)
        R.add("dash", R.rel(work / (name + ".txt")), R.rel(work / (name + ".raw")), W, H,
              int(round(w * 1000)), pat, c0, c1, dc, jn, int(round(off * 1000)), 1)
    res = R.run()
    fails = 0
    for c, line in zip(CASES, res):
        name, data, W, H, w, pat, c0, c1, dc, jn, off = c
        if not line.startswith("ok "):
            print(f"  FAIL {target} {name}: {line}")
            fails += 1
            continue
        img, _ = read_raw(work / (name + ".raw"))
        ref = reference(W, H, data, w, pat_of(pat), c0, c1, dc, jn, off)
        d = np.abs(img.astype(np.float64) - ref * 255)
        p995 = np.percentile(d, 99.5)
        # overlapping pieces (arrow legs over the line, square caps meeting at a corner):
        # the signed-area rasterizer adds partial coverages inside one pixel (min(sum, 1)),
        # so a few junction pixels are darker than the exact union
        tol = 72 if name in OVERLAP else 40
        if d.max() > tol or p995 > 20 or d.mean() > 0.6:
            ys, xs = np.nonzero(d > tol)
            print(f"  FAIL {target} {name}: max {d.max():.1f} p99.5 {p995:.1f} "
                  f"mean {d.mean():.3f} at {list(zip(xs.tolist(), ys.tolist()))[:6]}")
            fails += 1
    if a.bench:
        (work / "bench.txt").write_text("M 0 0 " + " ".join(
            "L %d %d" % (i * 7 % 1000, (i * 13) % 1000) for i in range(2000)))
        R.add("dash", R.rel(work / "bench.txt"), R.rel(work / "bench.raw"), 1000, 1000, 5000,
              "d3", 1, 3, 1, 1, 0, 5)
        print(f"[{target}] bench 2000 segments, dash-dot: {R.run()[0]}")
    print(f"[{target}] {len(CASES)} dash cases: {'ok' if not fails else str(fails) + ' failures'}")
    if not a.keep and not fails:
        shutil.rmtree(work, ignore_errors=True)
    return fails


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/t4")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--bench", action="store_true")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    fails = 0
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        fails += check_target(t, out, a)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
