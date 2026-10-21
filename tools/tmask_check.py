#!/usr/bin/env python3
"""Checks of the sparse tiled masks (gfx/tmask.s) against numpy references, driven through
apps/fxtest (runner in tools/blend_check.py).

  python tools/tmask_check.py --target both
  python tools/tmask_check.py --target win --only raster,trace --no-build

Suites: basic (alloc / load / store / clear / count_tiles sparsity / get / span / row /
row_w / tile_w / tile_set / fill_rect / invert / threshold / bbox / optimize / copy / clone),
combine (all five ops, incl. dst == src and combine_block with strides and sentinels; exact),
raster (tm_raster_polygon / tm_raster_path vs an exact area reference: max |d| <= 2; every op
bit-exact against the coverage it produced; chunking across 4096 px; polygons partly outside),
trace (contours: closed, rectilinear, corners only, every boundary edge exactly once with the
inside on the right, 4-connected saddles, early stop), huge (65535 x 65535 sparse masks: memory
stays small, timings).  Exit code 0 when everything passed.
"""

import argparse
import struct
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from blend_check import FxRunner, Rec, Report, ROOT  # noqa: E402
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402

(T_ALLOC, T_FREE, T_LOAD, T_STORE, T_COMBINE, T_INVERT, T_BBOX, T_THRESH, T_FILL, T_RPOLY,
 T_RPATH, T_TRACE, T_COUNT, T_SPAN, T_ROW, T_CLEAR, T_COPY, T_CLONE, T_GET, T_ROWW, T_OPT,
 T_TILESET, T_CBLOCK, T_TILEW) = range(20, 44)
OPS = ["replace", "add", "sub", "intersect", "xor"]
RULE_NONZERO, RULE_EVENODD = 0, 1


def div255r(t):
    return ((t + 128) * 257) >> 16


def ref_combine(d, s, op):
    d = d.astype(np.int64)
    s = s.astype(np.int64)
    if op == 0:
        r = s
    elif op == 1:
        r = np.maximum(d, s)
    elif op == 2:
        r = div255r(d * (255 - s))
    elif op == 3:
        r = div255r(d * s)
    else:
        r = np.abs(d - s)
    return r.astype(np.uint8)


def structured_mask(rng, w, h):
    """Random mask with every tile kind: empty, full, other constant, noise, partial."""
    m = np.zeros((h, w), np.uint8)
    for ty in range(0, h, 64):
        for tx in range(0, w, 64):
            k = rng.integers(0, 7)
            blk = m[ty:ty + 64, tx:tx + 64]
            if k == 1:
                blk[:] = 255
            elif k == 2:
                blk[:] = rng.integers(1, 255)
            elif k == 3:
                blk[:] = rng.integers(0, 256, blk.shape)
            elif k == 4:
                blk[:] = 255
                blk[rng.integers(0, blk.shape[0]), rng.integers(0, blk.shape[1])] = rng.integers(0, 255)
            elif k == 5:
                blk[: blk.shape[0] // 2] = 255
            elif k == 6:
                blk[:, rng.integers(0, blk.shape[1]):] = rng.choice([0, 255, 128])
    return m


def tile_kinds(m):
    """Expected (tiles in memory, full tiles) after normalisation."""
    h, w = m.shape
    tiles = full = 0
    for ty in range(0, h, 64):
        for tx in range(0, w, 64):
            blk = m[ty:ty + 64, tx:tx + 64]
            if (blk == 255).all():
                full += 1
            elif not (blk == 0).all():
                tiles += 1
    return tiles, full


class Masks:
    """Builds record lists for mask slots."""

    @staticmethod
    def alloc(slot, w, h):
        return Rec(T_ALLOC, [slot, w, h], [], f"alloc {slot} {w}x{h}")

    @staticmethod
    def load(slot, m):
        return Rec(T_LOAD, [slot, m.shape[1]], [(m.tobytes(), 0)], f"load {slot}")

    @staticmethod
    def store(slot):
        return Rec(T_STORE, [slot], [], f"store {slot}")


def stored(res, w, h):
    return np.frombuffer(res.extra[:w * h], np.uint8).reshape(h, w)


# ---------------------------------------------------------------- basic + combine

def suite_basic(run, rep, rng):
    t0 = time.time()
    for w, h in ((1, 1), (64, 64), (65, 63), (200, 130), (1000, 7), (7, 300), (128, 128)):
        a = structured_mask(rng, w, h)
        recs = [Masks.alloc(0, w, h), Rec(T_STORE, [0], [], "empty store"),
                Rec(T_COUNT, [0], [], "count empty"),
                Masks.load(0, a), Masks.store(0), Rec(T_COUNT, [0], [], "count"),
                Rec(T_BBOX, [0], [], "bbox")]
        res = run.run(recs)
        rep.check(res[0].res == 0, f"alloc {w}x{h}")
        rep.check(not stored(res[1], w, h).any(), f"new mask empty {w}x{h}")
        rep.check(struct.unpack("<II", res[2].extra) == (0, 0), f"empty count {w}x{h}")
        rep.check(np.array_equal(stored(res[4], w, h), a), f"load/store {w}x{h}")
        rep.check(struct.unpack("<II", res[5].extra) == tile_kinds(a), f"sparsity {w}x{h}",
                  f"{struct.unpack('<II', res[5].extra)} vs {tile_kinds(a)}")
        ys, xs = np.nonzero(a)
        exp = (1, xs.min(), ys.min(), xs.max() + 1, ys.max() + 1) if len(xs) else (0, 0, 0, 0, 0)
        got = (res[6].res,) + struct.unpack("<iiii", res[6].extra)
        rep.check(got == exp, f"bbox {w}x{h}", f"{got} vs {exp}")
        # get / span / row queries (after optimize, so tile kinds are canonical)
        recs, exps = [Masks.alloc(0, w, h), Masks.load(0, a), Rec(T_OPT, [0], [], "opt")], [None, None, None]
        for _ in range(300):
            x = int(rng.integers(-70, w + 70))
            y = int(rng.integers(-3, h + 3))
            n = int(rng.integers(1, 300))
            recs.append(Rec(T_GET, [0, x, y], [], f"get {x},{y}"))
            exps.append(("get", x, y))
            recs.append(Rec(T_SPAN, [0, x, y, n], [], f"span {x},{y},{n}"))
            exps.append(("span", x, y, n))
            recs.append(Rec(T_ROW, [0, x, y, n], [], f"row {x},{y},{n}"))
            exps.append(("row", x, y, n))
        res = run.run(recs)
        kinds = {}
        for ty in range(0, h, 64):
            for tx in range(0, w, 64):
                blk = a[ty:ty + 64, tx:tx + 64]
                kinds[(tx // 64, ty // 64)] = 0 if (blk == 0).all() else 1 if (blk == 255).all() else 2
        for r, rec, e in zip(res[3:], recs[3:], exps[3:]):
            x, y = e[1], e[2]
            inside = 0 <= x < w and 0 <= y < h
            if e[0] == "get":
                rep.check(r.res == (int(a[y, x]) if inside else 0), rec.name, f"got {r.res}")
            elif e[0] == "span":
                n = e[3]
                kind, cnt = struct.unpack_from("<II", r.extra)
                if not (0 <= y < h) or x >= w:
                    ok = (kind, cnt) == (0, n)
                elif x < 0:
                    ok = (kind, cnt) == (0, min(n, -x))
                else:
                    n = min(n, w - x)
                    k = kinds[(x // 64, y // 64)]
                    if k == 2:
                        ecnt = min(n, 64 - x % 64)
                        ok = (kind, cnt) == (2, ecnt) and r.extra[8:] == a[y, x:x + ecnt].tobytes()
                    else:
                        ecnt = 64 - x % 64
                        tx = x // 64 + 1
                        while ecnt < n and kinds.get((tx, y // 64)) == k:
                            ecnt += 64
                            tx += 1
                        ok = (kind, cnt) == (k, min(ecnt, n))
                rep.check(ok, rec.name, f"got {kind},{cnt}")
            else:
                n = e[3]
                row = np.zeros(n, np.uint8)
                if 0 <= y < h:
                    for i in range(n):
                        if 0 <= x + i < w:
                            row[i] = a[y, x + i]
                used_buf = struct.unpack_from("<I", r.extra)[0]
                in_tile = (inside and x + n <= w and x // 64 == (x + n - 1) // 64 and
                           kinds[(x // 64, y // 64)] == 2)
                rep.check(r.extra[4:] == row.tobytes() and used_buf == (0 if in_tile else 1),
                          rec.name, f"used_buf={used_buf}")
        # fill_rect / row_w / tile_w / tile_set / clear / invert / threshold / optimize
        m = a.copy()
        recs, exps = [Masks.alloc(0, w, h), Masks.load(0, a)], []
        for _ in range(12):
            x0, x1 = sorted(int(v) for v in rng.integers(-20, w + 20, 2))
            y0, y1 = sorted(int(v) for v in rng.integers(-20, h + 20, 2))
            v = int(rng.choice([0, 255, 17, 200]))
            recs.append(Rec(T_FILL, [0, x0, y0, x1, y1, v], [], f"fill {x0},{y0},{x1},{y1}={v}"))
            m[max(y0, 0):max(min(y1, h), 0), max(x0, 0):max(min(x1, w), 0)] = v
            recs.append(Masks.store(0))
            exps.append(m.copy())
            x = int(rng.integers(0, w))
            y = int(rng.integers(0, h))
            n = int(rng.integers(1, 150))
            v = int(rng.integers(0, 256))
            recs.append(Rec(T_ROWW, [0, x, y, n, v], [], f"row_w {x},{y},{n}"))
            m[y, x:min(x + n, w)] = v
            recs.append(Masks.store(0))
            exps.append(m.copy())
        tx, ty = int(rng.integers(0, (w + 63) // 64)), int(rng.integers(0, (h + 63) // 64))
        recs.append(Rec(T_TILESET, [0, tx, ty, 1], [], "tile_set full"))
        m[ty * 64:ty * 64 + 64, tx * 64:tx * 64 + 64] = 255
        recs.append(Masks.store(0))
        exps.append(m.copy())
        recs.append(Rec(T_TILEW, [0, tx, ty], [], "tile_w"))
        recs.append(Masks.store(0))
        exps.append(m.copy())
        recs.append(Rec(T_INVERT, [0], [], "invert"))
        m = 255 - m
        recs.append(Masks.store(0))
        exps.append(m.copy())
        for lvl in (1, 128, 255, 0, 999):
            recs.append(Rec(T_ALLOC, [1, w, h], [], "a1"))
            recs.append(Masks.load(1, m))
            recs.append(Rec(T_THRESH, [1, lvl], [], f"threshold {lvl}"))
            recs.append(Masks.store(1))
            L = min(max(lvl, 1), 255)
            exps.append(np.where(m >= L, 255, 0).astype(np.uint8))
        for v in (0, 255, 77):
            recs.append(Rec(T_CLEAR, [0, v], [], f"clear {v}"))
            recs.append(Masks.store(0))
            exps.append(np.full((h, w), v, np.uint8))
        recs.append(Rec(T_COUNT, [0], [], "count after clear 77"))
        recs.append(Rec(T_OPT, [0], [], "optimize"))
        recs.append(Rec(T_CLEAR, [0, 255], [], "clear 255"))
        recs.append(Rec(T_COUNT, [0], [], "count after clear 255"))
        res = run.run(recs)
        stores = [r for r, rc in zip(res, recs) if rc.op == T_STORE]
        for i, (r, e) in enumerate(zip(stores, exps)):
            got = stored(r, w, h)
            rep.check(np.array_equal(got, e), f"basic ops {w}x{h} step {i}",
                      f"{int((got != e).sum())} px differ")
        cnts = [struct.unpack("<II", r.extra) for r, rc in zip(res, recs) if rc.op == T_COUNT]
        nt = ((w + 63) // 64) * ((h + 63) // 64)
        rep.check(cnts[0] == (nt, 0), f"clear 77 materialises {w}x{h}", str(cnts[0]))
        rep.check(cnts[1] == (0, nt), f"clear 255 frees tiles {w}x{h}", str(cnts[1]))
        bad = [(rc.name, r.res) for r, rc in zip(res, recs) if rc.op in (T_FILL, T_ROWW, T_CLEAR) and r.res]
        rep.check(not bad, f"basic result codes {w}x{h}", str(bad[:3]))
        # copy / clone
        b = structured_mask(rng, w, h)
        res = run.run([Masks.alloc(0, w, h), Masks.load(0, a), Rec(T_ALLOC, [2, w, h], [], "a2"), Masks.load(2, b),
                       Rec(T_COPY, [2, 0], [], "copy"), Masks.store(2),
                       Rec(T_CLONE, [3, 2], [], "clone"), Masks.store(3), Rec(T_COUNT, [3], [], "c3")])
        rep.check(np.array_equal(stored(res[5], w, h), a), f"copy {w}x{h}")
        rep.check(res[6].res == 0 and np.array_equal(stored(res[7], w, h), a), f"clone {w}x{h}")
        rep.check(struct.unpack("<II", res[8].extra) == tile_kinds(a), f"clone sparsity {w}x{h}")
    # bad sizes
    res = run.run([Rec(T_ALLOC, [5, 0, 5], [], "0 w"), Rec(T_ALLOC, [5, 65536, 5], [], "65536"),
                   Rec(T_ALLOC, [5, 65535, 1], [], "65535 ok"), Rec(T_ALLOC, [6, 3, 3], [], "3x3"),
                   Rec(T_COMBINE, [5, 6, 1], [], "size mismatch")])
    rep.check([r.res for r in res] == [-1, -1, 0, 0, -1], "alloc limits / size mismatch",
              str([r.res for r in res]))
    rep.done("basic", t0)


def suite_combine(run, rep, rng):
    t0 = time.time()
    for w, h in ((1, 1), (200, 130), (64, 192), (333, 65)):
        for trial in range(3):
            a = structured_mask(rng, w, h)
            b = structured_mask(rng, w, h)
            recs, exps = [], []
            for op in range(5):
                recs += [Masks.alloc(0, w, h), Masks.load(0, a), Masks.alloc(1, w, h), Masks.load(1, b),
                         Rec(T_COMBINE, [0, 1, op], [], f"combine {OPS[op]} {w}x{h}"), Masks.store(0),
                         Rec(T_COUNT, [0], [], "count")]
                e = ref_combine(a, b, op)
                exps.append(e)
                recs += [Masks.load(0, a), Rec(T_COMBINE, [0, 0, op], [], f"self {OPS[op]}"), Masks.store(0),
                         Rec(T_COUNT, [0], [], "count")]
                exps.append(ref_combine(a, a, op))
            res = run.run(recs)
            stores = [(r, rc) for r, rc in zip(res, recs) if rc.op == T_STORE]
            counts = [r for r, rc in zip(res, recs) if rc.op == T_COUNT]
            combs = [r for r, rc in zip(res, recs) if rc.op == T_COMBINE]
            for i, ((r, rc), e, c, cb) in enumerate(zip(stores, exps, counts, combs)):
                got = stored(r, w, h)
                name = f"combine {OPS[i // 2]}{' self' if i % 2 else ''} {w}x{h} #{trial}"
                rep.check(cb.res == 0 and np.array_equal(got, e), name, f"{int((got != e).sum())} px differ")
                # uniform tiles must have become sentinels
                rep.check(struct.unpack("<II", c.extra) == tile_kinds(e), name + " sparsity",
                          f"{struct.unpack('<II', c.extra)} vs {tile_kinds(e)}")
    # combine_block with strides and sentinels
    w, h = 150, 100
    a = structured_mask(rng, w, h)
    recs, exps = [Masks.alloc(0, w, h), Masks.load(0, a)], []
    m = a.copy()
    for _ in range(40):
        tx, ty = int(rng.integers(0, 3)), int(rng.integers(0, 2))
        op = int(rng.integers(0, 5))
        mode = int(rng.integers(0, 3))
        stride = int(rng.choice([64, 80, 129]))
        blk = rng.integers(0, 256, (64, stride)).astype(np.uint8)
        kind = rng.integers(0, 4)
        if kind == 1:
            blk[:] = 0
        elif kind == 2:
            blk[:] = 255
        elif kind == 3:
            blk[:, :32] = 255
        src = blk[:, :64].astype(np.int64)
        if mode == 1:
            src = np.zeros((64, 64), np.int64)
        elif mode == 2:
            src = np.full((64, 64), 255, np.int64)
        y0, x0 = ty * 64, tx * 64
        reg = m[y0:y0 + 64, x0:x0 + 64]
        m[y0:y0 + 64, x0:x0 + 64] = ref_combine(reg, src[:reg.shape[0], :reg.shape[1]], op)
        recs.append(Rec(T_CBLOCK, [0, tx, ty, stride, op, mode], [(blk.tobytes(), 3)], f"cblock {op}"))
        recs.append(Masks.store(0))
        exps.append(m.copy())
    res = run.run(recs)
    stores = [r for r, rc in zip(res, recs) if rc.op == T_STORE]
    for i, (r, e) in enumerate(zip(stores, exps)):
        rep.check(np.array_equal(stored(r, w, h), e), f"combine_block step {i}")
    rep.done("combine", t0)


# ---------------------------------------------------------------- raster

def ref_coverage(polys, w, h, rule):
    """Exact area coverage (x 255) of closed, non-self-intersecting polygons: inside one pixel
    row the inside length of a scanline within every pixel column is linear in v between the
    breakpoints (vertex y's, y's where an edge crosses an integer x), so the midpoint rule on
    those intervals integrates exactly.  (Self-intersections add breakpoints that are ignored.)"""
    segs = []
    for p in polys:
        p = np.asarray(p, np.float64)
        q = np.roll(p, -1, axis=0)
        for (x0, y0), (x1, y1) in zip(p, q):
            if y0 != y1:
                segs.append((x0, y0, x1, y1))
    cov = np.zeros((h, w))
    if not segs:
        return cov
    S = np.array(segs)
    ymin = np.minimum(S[:, 1], S[:, 3])
    ymax = np.maximum(S[:, 1], S[:, 3])
    dirn = np.where(S[:, 3] > S[:, 1], 1, -1)
    grid = np.arange(w + 1, dtype=np.float64)
    for py in range(h):
        sel = (ymax > py) & (ymin < py + 1)
        if not sel.any():
            continue
        s, lo, hi, dr = S[sel], ymin[sel], ymax[sel], dirn[sel]
        ev = [float(py), float(py + 1)]
        ev += [v for v in np.concatenate((s[:, 1], s[:, 3])) if py < v < py + 1]
        for (x0, y0, x1, y1), a, b in zip(s, lo, hi):
            if x0 == x1:
                continue
            va, vb = max(a, py), min(b, py + 1)
            xa = x0 + (va - y0) * (x1 - x0) / (y1 - y0)
            xb = x0 + (vb - y0) * (x1 - x0) / (y1 - y0)
            k0 = max(int(np.floor(min(xa, xb))) + 1, 0)
            k1 = min(int(np.ceil(max(xa, xb))) - 1, w)
            if k1 >= k0:
                ks = np.arange(k0, k1 + 1, dtype=np.float64)
                ev += list(y0 + (ks - x0) * (y1 - y0) / (x1 - x0))
        ev = np.unique(np.clip(np.array(ev), py, py + 1))
        acc = np.zeros(w)
        for v0, v1 in zip(ev[:-1], ev[1:]):
            if v1 - v0 <= 0:
                continue
            v = 0.5 * (v0 + v1)
            m = (v >= lo) & (v < hi)
            if not m.any():
                continue
            ss = s[m]
            xs = ss[:, 0] + (v - ss[:, 1]) * (ss[:, 2] - ss[:, 0]) / (ss[:, 3] - ss[:, 1])
            order = np.argsort(xs, kind="stable")
            xs = xs[order]
            wind = np.cumsum(dr[m][order])
            inside = (wind != 0) if rule == RULE_NONZERO else (wind % 2 != 0)
            F = np.concatenate(([0.0], np.cumsum(np.diff(xs) * inside[:-1])))
            Fg = np.interp(grid, xs, F, left=0.0, right=F[-1])
            acc += np.diff(Fg) * (v1 - v0)
        cov[py] = acc
    return cov * 255.0


def poly_star(cx, cy, r0, r1, n, rot=0.0):
    a = np.arange(2 * n) * np.pi / n + rot
    r = np.where(np.arange(2 * n) % 2 == 0, r0, r1)
    return np.stack([cx + r * np.cos(a), cy + r * np.sin(a)], 1)


def poly_circle(cx, cy, r, n=64, rev=False):
    a = np.arange(n) * 2 * np.pi / n
    p = np.stack([cx + r * np.cos(a), cy + r * np.sin(a)], 1)
    return p[::-1] if rev else p


def path_blob(polys):
    parts = [struct.pack("<I", len(polys))]
    for p in polys:
        p = np.asarray(p, np.float32)
        parts.append(struct.pack("<I", len(p)))
        parts.append(p.tobytes())
    return b"".join(parts)


def suite_raster(run, rep, rng):
    t0 = time.time()
    cases = []
    w, h = 160, 120
    cases.append(("rect frac", w, h, [np.array([[10.25, 5.5], [100.75, 5.5], [100.75, 70.125], [10.25, 70.125]])], 0))
    cases.append(("triangle", w, h, [np.array([[3.3, 100.1], [150.2, 90.7], [60.6, 2.2]])], 0))
    cases.append(("star concave", w, h, [poly_star(80, 60, 55, 20, 7, 0.1)], 0))
    cases.append(("circle", w, h, [poly_circle(70.3, 50.6, 40.2, 90)], 1))
    cases.append(("partly outside", w, h, [np.array([[-30.5, -20.2], [140.4, 10.1], [190.3, 150.7], [20.0, 130.9]])], 0))
    cases.append(("ring evenodd", w, h, [poly_circle(80, 60, 50), poly_circle(80, 60, 25)], 1))
    cases.append(("ring nonzero hole", w, h, [poly_circle(80, 60, 50), poly_circle(80, 60, 25, rev=True)], 0))
    cases.append(("ring nonzero filled", w, h, [poly_circle(80, 60, 50), poly_circle(80, 60, 25)], 0))
    cases.append(("tiny", w, h, [np.array([[40.2, 40.3], [40.9, 40.35], [40.5, 40.95]])], 0))
    cases.append(("thin sliver", w, h, [np.array([[5.0, 5.0], [150.0, 7.0], [150.0, 7.3]])], 0))
    cases.append(("wide chunks", 5000, 70, [np.array([[3.5, 10.2], [4990.5, 3.3], [4200.25, 66.6], [12.0, 60.0]])], 0))
    cases.append(("tall", 70, 300, [poly_star(35, 150, 34, 10, 5)], 0))
    cases.append(("edge tiles", 130, 70, [np.array([[60.5, 30.5], [129.9, 40.2], [129.99, 69.99], [50, 69.9]])], 0))
    for i in range(6):
        n = int(rng.integers(3, 12))
        pts = np.stack([rng.uniform(-10, w + 10, n), rng.uniform(-10, h + 10, n)], 1)
        c = pts.mean(0)
        ang = np.arctan2(pts[:, 1] - c[1], pts[:, 0] - c[0])
        cases.append((f"random simple {i}", w, h, [pts[np.argsort(ang)]], int(rng.integers(0, 2))))
    for name, w, h, polys, rule in cases:
        exp = ref_coverage(polys, w, h, rule)
        pre = structured_mask(rng, w, h)
        recs = [Rec(T_ALLOC, [0, w, h], [], "a"), Masks.load(0, pre)]
        if len(polys) == 1:
            recs.append(Rec(T_RPOLY, [0, rule, 0], [(np.asarray(polys[0], np.float32).tobytes(), 0)], name))
        else:
            recs.append(Rec(T_RPATH, [0, rule, 0], [(path_blob(polys), 0)], name))
        recs.append(Masks.store(0))
        for op in range(1, 5):
            recs += [Masks.load(0, pre)]
            if len(polys) == 1:
                recs.append(Rec(T_RPOLY, [0, rule, op], [(np.asarray(polys[0], np.float32).tobytes(), 0)], name))
            else:
                recs.append(Rec(T_RPATH, [0, rule, op], [(path_blob(polys), 0)], name))
            recs += [Masks.store(0), Rec(T_COUNT, [0], [], "count")]
        res = run.run(recs)
        cov = stored(res[3], w, h).astype(np.int64)
        d = np.abs(cov - exp)
        rep.check(res[2].res == 0 and d.max() <= 2, f"raster {name} vs exact",
                  f"res={res[2].res} max |d| = {d.max():.2f} at {np.unravel_index(d.argmax(), d.shape)}")
        k = 4
        for op in range(1, 5):
            got = stored(res[k + 2], w, h)
            e = ref_combine(pre, cov.astype(np.uint8), op)
            rep.check(res[k + 1].res == 0 and np.array_equal(got, e), f"raster {name} op {OPS[op]}",
                      f"{int((got != e).sum())} px differ")
            rep.check(struct.unpack("<II", res[k + 3].extra) == tile_kinds(e), f"raster {name} op {OPS[op]} sparsity")
            k += 4
    # the draw context (target, clip, origin, opacity, colour) survives tm_raster_path
    tri = np.array([[1.5, 1.5], [30.2, 4.4], [9.9, 25.0]], np.float32)
    res = run.run([Rec(T_ALLOC, [0, 40, 30], [], "a"), Rec(44, [0], [(tri.tobytes(), 0)], "raster in context"),
                   Masks.store(0)])
    cov = stored(res[2], 40, 30).astype(np.int64)
    d = np.abs(cov - ref_coverage([tri], 40, 30, 0))
    rep.check(res[1].res == 0 and res[1].extra == struct.pack("<I", 1) and d.max() <= 2,
              "raster keeps the draw context", f"res {res[1].res} extra {res[1].extra.hex()} max |d| {d.max():.2f}")
    # self-intersecting star: rule semantics away from the crossings (loose)
    w, h = 160, 120
    star = poly_star(80, 60, 55, 55, 5) [[0, 4, 8, 2, 6]]
    for rule in (0, 1):
        exp = ref_coverage([star], w, h, rule)
        res = run.run([Rec(T_ALLOC, [0, w, h], [], "a"),
                       Rec(T_RPOLY, [0, rule, 0], [(star.astype(np.float32).tobytes(), 0)], "pentagram"),
                       Masks.store(0)])
        cov = stored(res[2], w, h).astype(np.int64)
        d = np.abs(cov - exp)
        rep.check(np.percentile(d, 99.5) <= 2 and d.mean() < 0.3, f"pentagram rule {rule}",
                  f"p99.5 {np.percentile(d, 99.5):.1f} mean {d.mean():.3f}")
    rep.done("raster", t0)


# ---------------------------------------------------------------- trace

def parse_contours(extra, count=None):
    out, p = [], 0
    while p < len(extra) - (8 if count else 0):
        n = struct.unpack_from("<I", extra, p)[0]
        p += 4
        pts = np.frombuffer(extra, np.int32, 2 * n, p).reshape(n, 2)
        p += 8 * n
        out.append(pts)
    return out


def expected_edges(b):
    """Directed unit boundary edges (inside on the right, y down) of a bool image."""
    h, w = b.shape
    pad = np.zeros((h + 2, w + 2), bool)
    pad[1:-1, 1:-1] = b
    edges = set()
    above, below = pad[:-1, 1:-1], pad[1:, 1:-1]          # lattice rows 0..h
    for y, x in zip(*np.nonzero(below & ~above)):
        edges.add(((x, y), (x + 1, y)))
    for y, x in zip(*np.nonzero(above & ~below)):
        edges.add(((x + 1, y), (x, y)))
    left, right = pad[1:-1, :-1], pad[1:-1, 1:]           # lattice columns 0..w
    for y, x in zip(*np.nonzero(left & ~right)):
        edges.add(((x, y), (x, y + 1)))
    for y, x in zip(*np.nonzero(right & ~left)):
        edges.add(((x, y + 1), (x, y)))
    return edges


def check_contours(rep, name, conts, b):
    h, w = b.shape
    got = []
    ok = True
    why = ""
    pad = np.zeros((h + 2, w + 2), bool)
    pad[1:-1, 1:-1] = b
    inside = lambda x, y: bool(pad[y + 1, x + 1])  # noqa: E731
    for c in conts:
        n = len(c)
        if n < 4:
            ok, why = False, f"contour with {n} points"
            break
        dirs = []
        for i in range(n):
            p, q = c[i], c[(i + 1) % n]
            dx, dy = int(q[0] - p[0]), int(q[1] - p[1])
            if (dx != 0) == (dy != 0):
                ok, why = False, f"non-rectilinear segment {p}->{q}"
                break
            sx, sy = np.sign(dx), np.sign(dy)
            dirs.append((sx, sy))
            for k in range(abs(dx) + abs(dy)):
                a = (int(p[0] + sx * k), int(p[1] + sy * k))
                got.append((a, (a[0] + sx, a[1] + sy)))
        if not ok:
            break
        for i in range(n):
            if dirs[i] == dirs[(i + 1) % n]:
                ok, why = False, f"collinear corner {c[(i + 1) % n]}"
                break
            # saddle rule: diagonal pixels are separate -> always a right turn at saddles
            vx, vy = int(c[(i + 1) % n][0]), int(c[(i + 1) % n][1])
            tl, tr, bl, br = inside(vx - 1, vy - 1), inside(vx, vy - 1), inside(vx - 1, vy), inside(vx, vy)
            if tl == br and tr == bl and tl != tr:
                d0, d1 = dirs[i], dirs[(i + 1) % n]
                right = (-d0[1], d0[0])                  # screen: (dx, dy) -> (-dy, dx)
                if d1 != right:
                    ok, why = False, f"saddle at {vx},{vy} not separated"
                    break
        if not ok:
            break
    if ok:
        exp = expected_edges(b)
        if len(got) != len(set(got)):
            ok, why = False, "edge traced twice"
        elif set(got) != exp:
            ok, why = False, f"edges differ: {len(set(got) - exp)} extra, {len(exp - set(got))} missing"
    rep.check(ok, f"trace {name}", why)


def suite_trace(run, rep, rng):
    t0 = time.time()
    cases = []
    for w, h in ((1, 1), (5, 4), (64, 64), (65, 65), (130, 70), (200, 129)):
        cases.append((f"empty {w}x{h}", np.zeros((h, w), np.uint8)))
        cases.append((f"full {w}x{h}", np.full((h, w), 255, np.uint8)))
        cases.append((f"noise {w}x{h}", rng.integers(0, 256, (h, w)).astype(np.uint8)))
        cases.append((f"structured {w}x{h}", structured_mask(rng, w, h)))
        yy, xx = np.mgrid[0:h, 0:w]
        blob = ((xx - w / 2) ** 2 + (yy - h / 2) ** 2 < (min(w, h) / 3) ** 2) & \
               ~((xx - w / 2) ** 2 + (yy - h / 2) ** 2 < (min(w, h) / 7) ** 2)
        cases.append((f"ring {w}x{h}", (blob * 200).astype(np.uint8)))
        cases.append((f"checker {w}x{h}", (((xx + yy) % 2) * 255).astype(np.uint8)))
        cases.append((f"sparse {w}x{h}", ((rng.random((h, w)) < 0.05) * 255).astype(np.uint8)))
        cases.append((f"level {w}x{h}", np.where(rng.random((h, w)) < 0.5, 127, 128).astype(np.uint8)))
    for name, m in cases:
        h, w = m.shape
        res = run.run([Rec(T_ALLOC, [0, w, h], [], "a"), Masks.load(0, m), Rec(T_TRACE, [0, 0], [], name)])
        rep.check(res[2].res == 0, f"trace {name} result", str(res[2].res))
        check_contours(rep, name, parse_contours(res[2].extra), m >= 128)
    # early stop
    m = ((rng.random((100, 100)) < 0.2) * 255).astype(np.uint8)
    res = run.run([Rec(T_ALLOC, [0, 100, 100], [], "a"), Masks.load(0, m), Rec(T_TRACE, [0, 3], [], "stop")])
    rep.check(res[2].res == 7 and len(parse_contours(res[2].extra)) == 3, "trace early stop",
              f"res {res[2].res}, {len(parse_contours(res[2].extra))} contours")
    rep.done("trace", t0)


# ---------------------------------------------------------------- huge sparse masks

def suite_huge(run, rep, rng):
    t0 = time.time()
    W = H = 65535
    tri = np.array([[60000.5, 60000.25], [60300.0, 60100.0], [60100.0, 60250.5]], np.float32)
    recs = [Rec(T_ALLOC, [0, W, H], [], "huge a"), Rec(T_ALLOC, [1, W, H], [], "huge b"),
            Rec(T_FILL, [0, 100, 200, 300, 250, 255], [], "fill"),
            Rec(T_FILL, [0, 65500, 65500, 70000, 70000, 128], [], "fill corner"),
            Rec(T_RPOLY, [0, 0, 1, 0, 0, 0, 0, 2], [(tri.tobytes(), 0)], "raster far"),
            Rec(T_BBOX, [0], [], "bbox"), Rec(T_COUNT, [0], [], "count"),
            Rec(T_TRACE, [0, 0, 0, 0, 0, 0, 0, 2], [], "trace huge"),
            Rec(T_FILL, [1, 0, 0, 65535, 65535, 255], [], "fill all"),
            Rec(T_COUNT, [1], [], "count full"),
            Rec(T_COMBINE, [1, 0, 2], [], "sub huge"),
            Rec(T_COUNT, [1], [], "count sub"),
            Rec(T_GET, [1, 150, 220], [], "get hole"), Rec(T_GET, [1, 5, 5], [], "get full"),
            Rec(T_INVERT, [1], [], "invert"), Rec(T_BBOX, [1], [], "bbox inv"),
            Rec(T_COUNT, [1], [], "count inv"),
            Rec(T_CLONE, [2, 1], [], "clone huge"), Rec(T_COUNT, [2], [], "count clone"),
            Rec(T_ROW, [1, 65400, 65520, 135], [], "row far"),
            Rec(T_CLEAR, [0, 0], [], "clear"), Rec(T_COUNT, [0], [], "count cleared")]
    res = run.run(recs)
    rep.check(all(r.res == 0 for r in res[:2]), "huge alloc")
    rep.check((res[5].res,) + struct.unpack("<iiii", res[5].extra) == (1, 100, 200, 65535, 65535),
              "huge bbox", str(struct.unpack("<iiii", res[5].extra)))
    tiles, full = struct.unpack("<II", res[6].extra)
    rep.check(tiles <= 40 and full <= 20, "huge sparsity", f"{tiles} tiles, {full} full")
    conts = parse_contours(res[7].extra, count=True)
    rep.check(res[7].res == 0 and len(conts) == 3, "huge trace", f"res {res[7].res}, {len(conts)} contours")
    rep.check(struct.unpack("<II", res[9].extra) == (0, 1024 * 1024), "huge full count")
    t2, f2 = struct.unpack("<II", res[11].extra)
    rep.check(res[10].res == 0 and t2 == tiles and f2 == 1024 * 1024 - tiles - full, "huge sub sparsity",
              f"{t2}, {f2}")
    rep.check(res[12].res == 0 and res[13].res == 255, "huge get", f"{res[12].res} {res[13].res}")
    rep.check((res[15].res,) + struct.unpack("<iiii", res[15].extra) == (1, 100, 200, 65535, 65535),
              "huge bbox inverted")
    rep.check(struct.unpack("<II", res[16].extra) == (tiles, full), "huge invert sparsity")
    rep.check(res[17].res == 0 and res[18].extra == res[16].extra, "huge clone",
              str(struct.unpack("<II", res[18].extra)))
    row = res[19].extra[4:]
    rep.check(row == bytes(100) + bytes([128]) * 35, "huge row")
    rep.check(struct.unpack("<II", res[21].extra) == (0, 0), "huge clear")
    print(f"[{rep.target}] huge 65535x65535: raster far triangle {res[4].time_us() / 2:.0f} us, "
          f"trace {res[7].time_us() / 2:.0f} us; {tiles} tiles in memory")
    # timing of a typical selection: 4000 x 3000 mask, 2000-point lasso, trace
    n = 2000
    a = np.linspace(0, 2 * np.pi, n, endpoint=False)
    r = 1300 + 200 * np.sin(a * 13)
    lasso = np.stack([2000 + r * np.cos(a) * 1.4, 1500 + r * np.sin(a)], 1).astype(np.float32)
    res = run.run([Rec(T_ALLOC, [0, 4000, 3000], [], "a"),
                   Rec(T_RPOLY, [0, 0, 0, 0, 0, 0, 0, 3], [(lasso.tobytes(), 0)], "lasso"),
                   Rec(T_COUNT, [0], [], "count"),
                   Rec(T_TRACE, [0, 0, 0, 0, 0, 0, 0, 3], [], "trace lasso")])
    print(f"[{rep.target}] 4000x3000 lasso ({n} points): raster {res[1].time_us() / 3000:.1f} ms, "
          f"trace {res[3].time_us() / 3000:.1f} ms, {struct.unpack('<II', res[2].extra)} (tiles, full)")
    rep.check(res[1].res == 0 and res[3].res == 0, "lasso raster/trace")
    rep.done("huge", t0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/g1")
    ap.add_argument("--only", default="", help="basic,combine,raster,trace,huge")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--seed", type=int, default=99)
    args = ap.parse_args()
    out = (ROOT / args.out).resolve()
    targets = ["win", "linux"] if args.target == "both" else [args.target]
    only = set(filter(None, args.only.split(",")))
    fails = 0
    for t in targets:
        if not args.no_build:
            build.build_app("fxtest", t, out)
        run = FxRunner(t, out)
        rng = np.random.default_rng(args.seed)
        for name, fn in (("basic", suite_basic), ("combine", suite_combine), ("raster", suite_raster),
                         ("trace", suite_trace), ("huge", suite_huge)):
            if not only or name in only:
                rep = Report(t)
                fn(run, rep, rng)
                fails += rep.fails
    print("tmask_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
