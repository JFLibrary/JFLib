#!/usr/bin/env python3
"""Bit-exact checks of the straight-alpha kernels (gfx/straight.s) and the paint.net blend
modes (gfx/blendmode.s) against numpy references, driven through apps/fxtest.

  python tools/blend_check.py --target both            # everything (builds fxtest first)
  python tools/blend_check.py --target win --only bm --no-bench
  python tools/blend_check.py --out build/g1 --no-build

Suites: straight (k_premul_row / k_unpremul_row exhaustive incl. the png.s reference and the
round-trip rules, k_lerp_mask_row, k_erase_row, k_view_row(_pm)), bm (every blend mode:
exhaustive 256 x 256 channel grid for many alpha pairs, 10^6 random pixels per mode with
random opacity, coverage and solid variants, BM_OVERWRITE), sizes (n = 0..67 at every
alignment 0..15, in place and separate, guard bytes around every buffer), bench (Mpx/s),
simd (V1: every kernel with `--simd sse2` vs the default dispatch (AVX2 where the CPU has
it) on mixed rows n = 0..67 and long rows, byte-identical outputs; plus the speed-up).
`--simd sse2` runs every suite on the SSE2 kernels.

The FxRunner class is shared with tools/tmask_check.py and tools/flood_check.py.
Exit code 0 when everything passed.
"""

import argparse
import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402

BM_NAMES = ["Normal", "Multiply", "Additive", "ColorBurn", "ColorDodge", "Reflect", "Glow",
            "Overlay", "Difference", "Negation", "Lighten", "Darken", "Screen", "Xor",
            "Overwrite"]
BM_OVERWRITE = 14

OP_PREMUL, OP_UNPREMUL, OP_PNGUNPM, OP_LERP, OP_ERASE, OP_VIEW, OP_VIEWPM = 1, 2, 3, 4, 5, 6, 7
OP_BM, OP_BMCOV, OP_BMSOLID = 10, 11, 12
PAGE_END = 0xFFFFFFFF


# ---------------------------------------------------------------- runner

class Rec:
    """One fxtest record: op, params (<= 16 ints), buffers [(bytes, align)]."""
    __slots__ = ("op", "params", "bufs", "name")

    def __init__(self, op, params, bufs, name=""):
        self.op, self.params, self.name = op, list(params), name
        self.bufs = [(bytes(b), a) for b, a in bufs]

    def encode(self):
        p = [struct.pack("<I", len(self.params))]
        p += [struct.pack("<i", int(v) if int(v) < 2**31 else int(v) - 2**32) for v in self.params]
        p.append(struct.pack("<I", len(self.bufs)))
        for b, a in self.bufs:
            p.append(struct.pack("<II", len(b), a & 0xFFFFFFFF))
            p.append(b)
        payload = b"".join(p)
        return struct.pack("<II", self.op, len(payload)) + payload


class Result:
    __slots__ = ("res", "bufs", "extra")

    def __init__(self, res, bufs, extra):
        self.res, self.bufs, self.extra = res, bufs, extra

    def time_us(self):
        return struct.unpack_from("<Q", self.extra, len(self.extra) - 8)[0]


class FxRunner:
    def __init__(self, target, out, extra=()):
        self.target, self.out, self.extra = target, out, list(extra)
        self.exe = build.exe_path("fxtest", target, out)
        self.work = out / target / "fx"
        self.work.mkdir(parents=True, exist_ok=True)
        self.seq = 0

    def rel(self, p):
        return str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")

    def run(self, recs, timeout=1800):
        """Run the records in one process; returns a list of Result."""
        if not recs:
            return []
        self.seq += 1
        bin_ = self.work / f"b{self.seq}.bin"
        res = self.work / f"b{self.seq}.res"
        bin_.write_bytes(b"FXB1" + b"".join(r.encode() for r in recs))
        if res.exists():
            res.unlink()
        dummy = self.rel(self.work / "unused.png")
        cmd = build.run_cmd_for(self.exe, self.target,
                                ["--shot", dummy, "--batch", self.rel(bin_), "--result", self.rel(res)]
                                + self.extra)
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=timeout)
        if p.returncode != 0 or not res.exists():
            raise RuntimeError(f"fxtest failed (exit {p.returncode}) on {len(recs)} records "
                               f"[{recs[0].name} ...]:\n{p.stdout}{p.stderr}")
        blob = res.read_bytes()
        bin_.unlink()
        res.unlink()
        if blob[:4] != b"FXR1":
            raise RuntimeError("bad result magic")
        out, pos = [], 4
        for r in recs:
            code, ln = struct.unpack_from("<iI", blob, pos)
            pos += 8
            data = blob[pos:pos + ln]
            pos += ln
            bufs, q = [], 0
            for b, _ in r.bufs:
                bufs.append(data[q:q + len(b)])
                q += len(b)
            out.append(Result(code, bufs, data[q:]))
        if pos != len(blob):
            raise RuntimeError("result size mismatch")
        return out


class Report:
    def __init__(self, target):
        self.target, self.fails, self.checks = target, 0, 0

    def check(self, ok, what, detail=""):
        self.checks += 1
        if not ok:
            self.fails += 1
            print(f"  [{self.target}] FAIL {what} {detail}")
        return ok

    def done(self, suite, t0):
        print(f"[{self.target}] {suite}: {self.checks} checks, {self.fails} failures "
              f"({time.time() - t0:.1f} s)")


def px_bytes(a):
    return np.ascontiguousarray(a, dtype=np.uint32).tobytes()


def from_bytes(b):
    return np.frombuffer(b, dtype=np.uint32).copy()


def split(p):
    p = p.astype(np.int64)
    return p & 255, (p >> 8) & 255, (p >> 16) & 255, (p >> 24) & 255


def join(b, g, r, a):
    return (np.asarray(b, np.int64) | (np.asarray(g, np.int64) << 8) |
            (np.asarray(r, np.int64) << 16) | (np.asarray(a, np.int64) << 24)).astype(np.uint32)


def first_diff(got, exp, *ctx):
    idx = np.nonzero(got != exp)[0]
    if len(idx) == 0:
        return ""
    i = idx[0]
    extra = " ".join(f"{c[i]:#010x}" if hasattr(c, "__len__") else str(c) for c in ctx)
    return f"{len(idx)} differ, first at {i}: got {int(got[i]):#010x} exp {int(exp[i]):#010x} {extra}"


# ---------------------------------------------------------------- references

def int_scale(a, b):
    t = a * b + 0x80
    return ((t >> 8) + t) >> 8


def div255r(t):
    """round(t / 255) the way the kernels compute it: ((t + 128) * 257) >> 16."""
    return ((t + 128) * 257) >> 16


def blend_f(mode, l, r):
    if mode == 0:
        return r
    if mode == 1:
        return int_scale(l, r)
    if mode == 2:
        return np.minimum(255, l + r)
    if mode == 3:
        q = ((255 - l) * 255) // np.maximum(r, 1)
        return np.where(r == 0, 0, np.maximum(0, 255 - q))
    if mode == 4:
        q = (l * 255) // np.maximum(255 - r, 1)
        return np.where(r == 255, 255, np.minimum(255, q))
    if mode == 5:
        q = (l * l) // np.maximum(255 - r, 1)
        return np.where(r == 255, 255, np.minimum(255, q))
    if mode == 6:
        q = (r * r) // np.maximum(255 - l, 1)
        return np.where(l == 255, 255, np.minimum(255, q))
    if mode == 7:
        return np.where(l < 128, int_scale(2 * l, r), 255 - int_scale(2 * (255 - l), 255 - r))
    if mode == 8:
        return np.abs(l - r)
    if mode == 9:
        return 255 - np.abs(255 - l - r)
    if mode == 10:
        return np.maximum(l, r)
    if mode == 11:
        return np.minimum(l, r)
    if mode == 12:
        return r + l - int_scale(r, l)
    if mode == 13:
        return l ^ r
    raise ValueError(mode)


def ref_blend(lhs, rhs, mode, opacity, cov=None):
    """paint.net UserBlendOps on straight BGRA (uint32 arrays) -> uint32 array."""
    lb, lg, lr, la = split(lhs)
    rb, rg, rr, ra = split(rhs)
    c = None if cov is None else np.asarray(cov, np.int64)
    if mode == BM_OVERWRITE:
        m = int_scale(np.full(la.shape, 255, np.int64) if c is None else c, opacity)
        chans = [div255r(lv * (255 - m) + rv * m) for lv, rv in
                 ((lb, rb), (lg, rg), (lr, rr), (la, ra))]
        return join(*chans)
    rA = int_scale(ra, opacity)
    if c is not None:
        rA = int_scale(rA, c)
    y = int_scale(la, 255 - rA)
    tot = y + rA
    x = int_scale(la, rA)
    z = rA - x
    d = np.maximum(tot, 1)
    out = []
    for lv, rv in ((lb, rb), (lg, rg), (lr, rr)):
        out.append((lv * y + rv * z + blend_f(mode, lv, rv) * x) // d)
    res = join(out[0], out[1], out[2], tot)
    res = np.where(tot == 0, np.uint32(0), res)
    if c is not None:
        res = np.where(c == 0, lhs, res)
    return res.astype(np.uint32)


def ref_premul(p):
    b, g, r, a = split(p)
    return join(div255r(b * a), div255r(g * a), div255r(r * a), a)


RCP = [0] + [0xFEFFFFFF // a + 1 for a in range(1, 256)]


def ref_unpremul(p):
    b, g, r, a = split(p)
    rcp = np.array(RCP, np.int64)[a]
    ch = [np.minimum(255, (v * rcp + (1 << 23)) >> 24) for v in (b, g, r)]
    res = join(ch[0], ch[1], ch[2], a)
    res = np.where(a == 0, np.uint32(0), res)
    res = np.where(a == 255, p, res)
    return res.astype(np.uint32)


def ref_lerp(a, b, m):
    m = np.asarray(m, np.int64)
    ca, cb = split(a), split(b)
    return join(*[div255r(x * (255 - m) + y * m) for x, y in zip(ca, cb)])


def ref_erase(p, cov, strength):
    b, g, r, a = split(p)
    e = div255r(np.asarray(cov, np.int64) * strength)
    return join(b, g, r, div255r(a * (255 - e)))


def checker(n, x0, y0, cell, c0, c1):
    xs = np.arange(n, dtype=np.int64) + x0
    par = (np.floor_divide(xs, cell) + (y0 // cell)) & 1
    return np.where(par == 1, np.int64(c1 | 0xFF000000), np.int64(c0 | 0xFF000000))


def ref_view(src, x0, y0, cell, c0, c1, pm):
    bg = checker(len(src), x0, y0, cell, c0, c1)
    sb, sg, sr, sa = split(src)
    bb, bgc, br, _ = split(bg.astype(np.uint32))
    if pm:
        ch = [s + div255r(k * (255 - sa)) for s, k in ((sb, bb), (sg, bgc), (sr, br))]
        ch = [np.minimum(v, 255) for v in ch]
    else:
        ch = [div255r(s * sa + k * (255 - sa)) for s, k in ((sb, bb), (sg, bgc), (sr, br))]
    return join(ch[0], ch[1], ch[2], np.full(len(src), 255, np.int64))


# ---------------------------------------------------------------- suites

def grid_pixels(la, ra):
    """65536 pixel pairs covering every (lhs.c, rhs.c) combination in each of B, G, R."""
    l = np.repeat(np.arange(256, dtype=np.int64), 256)
    r = np.tile(np.arange(256, dtype=np.int64), 256)
    lhs = join(l, r, 255 - l, np.full(65536, la, np.int64))
    rhs = join(r, l, 255 - r, np.full(65536, ra, np.int64))
    return lhs, rhs


ALPHA_PAIRS = [(255, 255), (255, 0), (0, 255), (0, 0), (255, 128), (128, 255), (1, 1), (1, 254),
               (254, 1), (100, 200), (200, 100), (37, 255), (255, 37), (128, 128), (64, 192),
               (3, 250), (250, 3), (127, 129), (2, 2), (255, 1)]


def suite_bm(run, rep, rng, modes):
    t0 = time.time()
    for mode in modes:
        recs, exps = [], []
        # exhaustive channel grid for many alpha pairs (opacity 255, no coverage)
        for la, ra in ALPHA_PAIRS:
            lhs, rhs = grid_pixels(la, ra)
            recs.append(Rec(OP_BM, [65536, mode | 255 << 8, 0],
                            [(bytes(65536 * 4), 0), (px_bytes(lhs), 4), (px_bytes(rhs), 8)],
                            f"bm grid {BM_NAMES[mode]} {la},{ra}"))
            exps.append(ref_blend(lhs, rhs, mode, 255))
        # random pixels, random opacity, all three kernels
        n = 1000000 if mode != BM_OVERWRITE else 250000
        for opac in (255, 0, 1, 77, 128, 254, int(rng.integers(0, 256))):
            lhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
            rhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
            # a quarter of the alphas at 0 / 255 to hit the fast paths
            for arr in (lhs, rhs):
                k = rng.integers(0, 4, n)
                arr[k == 0] |= 0xFF000000
                arr[k == 1] &= 0x00FFFFFF
            recs.append(Rec(OP_BM, [n, mode | opac << 8, 1],
                            [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 4)],
                            f"bm rand {BM_NAMES[mode]} op {opac}"))
            exps.append(("inplace", ref_blend(lhs, rhs, mode, opac)))
            cov = rng.integers(0, 256, n).astype(np.int64)
            k = rng.integers(0, 3, n)
            cov[k == 0] = 0
            cov[k == 1] = 255
            m = min(n, 300000)
            recs.append(Rec(OP_BMCOV, [m, mode | opac << 8, 0, 0],
                            [(bytes(m * 4), 12), (px_bytes(lhs[:m]), 0), (px_bytes(rhs[:m]), 4),
                             (cov[:m].astype(np.uint8).tobytes(), 3)],
                            f"bm cov {BM_NAMES[mode]} op {opac}"))
            exps.append(ref_blend(lhs[:m], rhs[:m], mode, opac, cov[:m]))
            argb = int(rng.integers(0, 2**32))
            if opac == 255:
                argb |= 0xFF000000
            recs.append(Rec(OP_BMSOLID, [m, mode | opac << 8, 1, 0, argb],
                            [(bytes(4), 0), (px_bytes(lhs[:m]), 0),
                             (cov[:m].astype(np.uint8).tobytes(), 0)],
                            f"bm solid {BM_NAMES[mode]} op {opac}"))
            exps.append(("inplace", ref_blend(lhs[:m], np.full(m, argb, np.uint32), mode, opac,
                                              cov[:m])))
            recs.append(Rec(OP_BMSOLID, [m, mode | opac << 8, 0, 1, argb],
                            [(bytes(m * 4), 0), (px_bytes(lhs[:m]), 0), (bytes(1), 0)],
                            f"bm solid nocov {BM_NAMES[mode]} op {opac}"))
            exps.append(ref_blend(lhs[:m], np.full(m, argb, np.uint32), mode, opac,
                                  np.full(m, 255)))
        res = run.run(recs)
        for r, rec, e in zip(res, recs, exps):
            if isinstance(e, tuple):
                got = from_bytes(r.bufs[1])
                e = e[1]
            else:
                got = from_bytes(r.bufs[0])
            rep.check(r.res == 0 and np.array_equal(got, e), rec.name,
                      f"res={r.res} " + first_diff(got, e))
    rep.done("bm", t0)


def suite_sizes(run, rep, rng, modes):
    """Every n = 0..67 at every alignment: SIMD body vs tails, guard bytes."""
    t0 = time.time()
    recs, exps = [], []
    for mode in modes:
        for n in range(68):
            for al in range(16):
                lhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
                rhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
                sel = rng.integers(0, 3, n)
                rhs[sel == 0] |= 0xFF000000
                lhs[sel == 1] |= 0xFF000000
                cov = rng.integers(0, 256, n)
                cov[rng.integers(0, 3, n) == 0] = 0
                opac = int(rng.choice([255, 255, 128, 3]))
                align = PAGE_END if al == 15 and n % 3 == 0 else al
                if (n + al) % 2:
                    recs.append(Rec(OP_BM, [n, mode | opac << 8, 0],
                                    [(bytes(4 * n), align), (px_bytes(lhs), (al * 7) % 16),
                                     (px_bytes(rhs), PAGE_END)], f"size bm {BM_NAMES[mode]} n={n} al={al}"))
                    exps.append((0, ref_blend(lhs, rhs, mode, opac)))
                else:
                    recs.append(Rec(OP_BMCOV, [n, mode | opac << 8, 1, 0],
                                    [(bytes(4), 0), (px_bytes(lhs), align), (px_bytes(rhs), (al * 5) % 16),
                                     (cov.astype(np.uint8).tobytes(), PAGE_END)],
                                    f"size bmcov {BM_NAMES[mode]} n={n} al={al}"))
                    exps.append((1, ref_blend(lhs, rhs, mode, opac, cov)))
                if al % 4 == 0:
                    argb = int(rng.integers(0, 2**32))
                    recs.append(Rec(OP_BMSOLID, [n, mode | opac << 8, 0, 0, argb],
                                    [(bytes(4 * n), (al + 3) % 16), (px_bytes(lhs), PAGE_END),
                                     (cov.astype(np.uint8).tobytes(), al)],
                                    f"size solid {BM_NAMES[mode]} n={n} al={al}"))
                    exps.append((0, ref_blend(lhs, np.full(n, argb, np.uint32), mode, opac, cov)))
    for n in range(68):
        for al in range(16):
            align = PAGE_END if al == 15 else al
            p = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
            p[rng.integers(0, 3, n) == 0] |= 0xFF000000
            q = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
            m = rng.integers(0, 256, n)
            m[rng.integers(0, 3, n) == 0] = 0
            m[rng.integers(0, 3, n) == 0] = 255
            recs.append(Rec(OP_PREMUL, [n, 0], [(bytes(4 * n), align), (px_bytes(p), (al * 3) % 16)],
                            f"size premul n={n} al={al}"))
            exps.append((0, ref_premul(p)))
            recs.append(Rec(OP_UNPREMUL, [n, 1], [(bytes(4), 0), (px_bytes(p), align)],
                            f"size unpremul n={n} al={al}"))
            exps.append((1, ref_unpremul(p)))
            recs.append(Rec(OP_LERP, [n, al % 3], [(bytes(4 * n), align), (px_bytes(p), (al + 1) % 16),
                                                  (px_bytes(q), (al + 2) % 16),
                                                  (m.astype(np.uint8).tobytes(), PAGE_END)],
                            f"size lerp n={n} al={al}"))
            exps.append(({0: 0, 1: 1, 2: 2}[al % 3], ref_lerp(p, q, m)))
            recs.append(Rec(OP_ERASE, [n, (al * 37) % 256], [(px_bytes(p), align),
                                                             (m.astype(np.uint8).tobytes(), (al * 3) % 16)],
                            f"size erase n={n} al={al}"))
            exps.append((0, ref_erase(p, m, (al * 37) % 256)))
            cfg = struct.pack("<iIII", 1 + al % 9, 0x00C0C0C0, 0x00FFFFFF, 0)
            recs.append(Rec(OP_VIEW if al % 2 else OP_VIEWPM, [n, al * 3 - 20, al - 7],
                            [(bytes(4 * n), align), (px_bytes(p if al % 2 else ref_premul(p)), (al * 5) % 16),
                             (cfg, 0)], f"size view n={n} al={al}"))
            exps.append((0, ref_view(p if al % 2 else ref_premul(p), al * 3 - 20, al - 7, 1 + al % 9,
                                     0xC0C0C0, 0xFFFFFF, not al % 2)))
    res = run.run(recs)
    for r, rec, (bi, e) in zip(res, recs, exps):
        got = from_bytes(r.bufs[bi])
        rep.check(r.res == 0 and np.array_equal(got, e), rec.name, f"res={r.res} " + first_diff(got, e))
    rep.done("sizes", t0)


def suite_straight(run, rep, rng):
    t0 = time.time()
    # every (c, a) combination, c in all four positions
    c = np.repeat(np.arange(256, dtype=np.int64), 256)
    a = np.tile(np.arange(256, dtype=np.int64), 256)
    straight = join(c, 255 - c, c ^ 0x5A, a)
    premul_all = join(c, (c * 7) & 255, 255 - c, a)             # includes invalid c > a
    recs = [Rec(OP_PREMUL, [65536, 0], [(bytes(65536 * 4), 0), (px_bytes(straight), 0)], "premul all"),
            Rec(OP_UNPREMUL, [65536, 0], [(bytes(65536 * 4), 0), (px_bytes(premul_all), 0)], "unpremul all"),
            Rec(OP_PNGUNPM, [65536, 0], [(bytes(65536 * 4), 0), (px_bytes(premul_all), 0)], "png unpremul all")]
    res = run.run(recs)
    got_pm = from_bytes(res[0].bufs[0])
    rep.check(np.array_equal(got_pm, ref_premul(straight)), "k_premul_row exhaustive",
              first_diff(got_pm, ref_premul(straight)))
    got_um = from_bytes(res[1].bufs[0])
    got_png = from_bytes(res[2].bufs[0])
    rep.check(np.array_equal(got_um, got_png), "k_unpremul_row == png.s unpremul_row",
              first_diff(got_um, got_png))
    rep.check(np.array_equal(got_um, ref_unpremul(premul_all)), "k_unpremul_row exhaustive",
              first_diff(got_um, ref_unpremul(premul_all)))
    # round-trip rules on all (c, a):  premul(unpremul(p)) == p for every valid premultiplied
    # p (c <= a); unpremul(premul(s)) == s exactly when a == 255, else |d| <= ceil(128 / a)
    valid = join(np.minimum(c, a), np.minimum(c, a), np.minimum(255 - c, a), a)
    recs = [Rec(OP_UNPREMUL, [65536, 1], [(bytes(4), 0), (px_bytes(valid), 0)], "rt1"),
            Rec(OP_PREMUL, [65536, 1], [(bytes(4), 0), (px_bytes(straight), 0)], "rt2")]
    res = run.run(recs)
    back = from_bytes(res[0].bufs[1])
    res2 = run.run([Rec(OP_PREMUL, [65536, 1], [(bytes(4), 0), (px_bytes(back), 0)], "rt1b")])
    again = from_bytes(res2[0].bufs[1])
    rep.check(np.array_equal(again, valid), "premul(unpremul(p)) == p for c <= a",
              first_diff(again, valid))
    pm = from_bytes(res[1].bufs[1])
    res3 = run.run([Rec(OP_UNPREMUL, [65536, 1], [(bytes(4), 0), (px_bytes(pm), 0)], "rt2b")])
    rt = from_bytes(res3[0].bufs[1])
    sb, sg, sr, sa = split(straight)
    tb, tg, tr, ta = split(rt)
    lim = np.where(sa == 0, 255, -(-128 // np.maximum(sa, 1)))
    ok = (ta == sa) & ((sa == 0) | ((np.abs(tb - sb) <= lim) & (np.abs(tg - sg) <= lim) &
                                    (np.abs(tr - sr) <= lim)))
    ok &= np.where(sa == 255, rt == straight, True)
    rep.check(bool(ok.all()), "unpremul(premul(s)): exact at a = 255, |d| <= ceil(128 / a)",
              f"{int((~ok).sum())} violations")
    # lerp: every (a, b, m) triple (four channels per pixel)
    recs, exps = [], []
    for m0 in range(0, 256, 32):
        mm = np.arange(m0, m0 + 32, dtype=np.int64)
        ab = np.arange(65536, dtype=np.int64)
        A = np.tile(ab & 255, 32)
        B = np.tile(ab >> 8, 32)
        M = np.repeat(mm, 65536)
        pa = join(A, B, 255 - A, A ^ B)
        pb = join(B, A, 255 - B, (A * 3) & 255)
        recs.append(Rec(OP_LERP, [len(M), 0], [(bytes(len(M) * 4), 0), (px_bytes(pa), 0), (px_bytes(pb), 0),
                                              (M.astype(np.uint8).tobytes(), 0)], f"lerp m {m0}"))
        exps.append(ref_lerp(pa, pb, M))
    # erase: every (alpha, cov) for many strengths
    for s in (0, 1, 2, 64, 127, 128, 129, 200, 254, 255):
        al = np.repeat(np.arange(256, dtype=np.int64), 256)
        cv = np.tile(np.arange(256, dtype=np.int64), 256)
        p = join(cv, al, 255 - cv, al)
        recs.append(Rec(OP_ERASE, [65536, s], [(px_bytes(p), 0), (cv.astype(np.uint8).tobytes(), 0)],
                        f"erase s={s}"))
        exps.append(ref_erase(p, cv, s))
    res = run.run(recs)
    for r, rec, e in zip(res, recs, exps):
        got = from_bytes(r.bufs[0])
        rep.check(np.array_equal(got, e), rec.name, first_diff(got, e))
    # view: random pixels, several checkerboards and offsets
    recs, exps = [], []
    for cell in (1, 2, 3, 8, 10, 16):
        for x0, y0 in ((0, 0), (-37, -5), (13, 1000), (-1, 7)):
            n = 5000
            p = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
            p[rng.integers(0, 3, n) == 0] |= 0xFF000000
            p[rng.integers(0, 4, n) == 0] &= 0x00FFFFFF
            cfg = struct.pack("<iIII", cell, 0x00FFFFFF, 0x00CCCCCC, 0)
            recs.append(Rec(OP_VIEW, [n, x0, y0], [(bytes(4 * n), 0), (px_bytes(p), 0), (cfg, 0)],
                            f"view cell {cell} {x0},{y0}"))
            exps.append(ref_view(p, x0, y0, cell, 0xFFFFFF, 0xCCCCCC, False))
            q = ref_premul(p)
            recs.append(Rec(OP_VIEWPM, [n, x0, y0], [(bytes(4 * n), 0), (px_bytes(q), 0), (cfg, 0)],
                            f"view_pm cell {cell} {x0},{y0}"))
            exps.append(ref_view(q, x0, y0, cell, 0xFFFFFF, 0xCCCCCC, True))
    res = run.run(recs)
    for r, rec, e in zip(res, recs, exps):
        got = from_bytes(r.bufs[0])
        rep.check(np.array_equal(got, e), rec.name, first_diff(got, e))
    rep.done("straight", t0)


def suite_bench(run, rep, rng):
    t0 = time.time()
    n = 1 << 20
    reps = 30
    lhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
    rhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
    opaque_l = lhs | 0xFF000000
    opaque_r = rhs | 0xFF000000
    cov = rng.integers(0, 256, n).astype(np.uint8)
    recs = []
    for mode in range(15):
        recs.append(Rec(OP_BM, [n, mode | 255 << 8, 1, 0, 0, 0, 0, reps],
                        [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 0)], f"rand {mode}"))
        recs.append(Rec(OP_BM, [n, mode | 255 << 8, 1, 0, 0, 0, 0, reps],
                        [(bytes(4), 0), (px_bytes(opaque_l), 0), (px_bytes(opaque_r), 0)], f"opaque {mode}"))
        recs.append(Rec(OP_BMCOV, [n, mode | 200 << 8, 1, 0, 0, 0, 0, reps],
                        [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 0), (cov.tobytes(), 0)],
                        f"cov {mode}"))
    for op, name in ((OP_PREMUL, "k_premul_row"), (OP_UNPREMUL, "k_unpremul_row")):
        recs.append(Rec(op, [n, 1, 0, 0, 0, 0, 0, reps], [(bytes(4), 0), (px_bytes(lhs), 0)], name))
    recs.append(Rec(OP_LERP, [n, 1, 0, 0, 0, 0, 0, reps],
                    [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 0), (cov.tobytes(), 0)], "k_lerp_mask_row"))
    cfg = struct.pack("<iIII", 8, 0xFFFFFF, 0xCCCCCC, 0)
    recs.append(Rec(OP_VIEW, [n, 0, 0, 0, 0, 0, 0, reps],
                    [(bytes(4 * n), 0), (px_bytes(lhs), 0), (cfg, 0)], "k_view_row"))
    res = run.run(recs)
    mpx = lambda r: n * reps / max(r.time_us(), 1)  # noqa: E731
    lines = []
    for mode in range(15):
        a, b, c = res[3 * mode], res[3 * mode + 1], res[3 * mode + 2]
        lines.append(f"    {BM_NAMES[mode]:<11} random {mpx(a):7.0f}  opaque {mpx(b):7.0f}  "
                     f"cov+opacity {mpx(c):7.0f} Mpx/s")
        need = 150 if mode == 0 else 40
        rep.check(mpx(a) >= need, f"bench {BM_NAMES[mode]} >= {need} Mpx/s", f"{mpx(a):.0f}")
    names = ["k_premul_row", "k_unpremul_row", "k_lerp_mask_row", "k_view_row"]
    for nm, r in zip(names, res[45:]):
        lines.append(f"    {nm:<16} {mpx(r):7.0f} Mpx/s (random data)")
    print(f"[{rep.target}] bench (1 Mpx rows x {reps}):")
    print("\n".join(lines))
    rep.done("bench", t0)


def mixed_px(rng, n):
    """Pixels in runs of opaque / transparent / random alpha (exercises every fast path at
    any grouping)."""
    p = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
    i = 0
    while i < n:
        k = int(rng.integers(1, 10))
        c = int(rng.integers(0, 4))
        if c == 0:
            p[i:i + k] |= 0xFF000000
        elif c == 1:
            p[i:i + k] &= 0x00FFFFFF
        i += k
    return p


def mixed_cov(rng, n):
    c = rng.integers(0, 256, n)
    i = 0
    while i < n:
        k = int(rng.integers(1, 10))
        v = int(rng.integers(0, 4))
        if v < 2:
            c[i:i + k] = 255 * v
        i += k
    return c.astype(np.uint8)


def suite_simd(run, rep, rng, modes):
    """--simd sse2 vs default dispatch: byte-identical results; speed-up of the dispatch."""
    t0 = time.time()
    recs = []
    sizes = list(range(68)) + [1000, 4099]
    for mode in modes:
        for n in sizes:
            al = int(rng.integers(0, 16))
            lhs, rhs = mixed_px(rng, n), mixed_px(rng, n)
            cov = mixed_cov(rng, n)
            opac = int(rng.choice([255, 255, 128, 3]))
            recs.append(Rec(OP_BM, [n, mode | opac << 8, n % 2],
                            [(bytes(4 * n), al), (px_bytes(lhs), (al * 7) % 16),
                             (px_bytes(rhs), PAGE_END)], f"simd bm {BM_NAMES[mode]} n={n}"))
            recs.append(Rec(OP_BMCOV, [n, mode | opac << 8, 1, n % 3 == 0],
                            [(bytes(4), 0), (px_bytes(lhs), al), (px_bytes(rhs), (al * 5) % 16),
                             (cov.tobytes(), PAGE_END)], f"simd bmcov {BM_NAMES[mode]} n={n}"))
            argb = int(rng.choice([0xFF000000, 0, int(rng.integers(0, 2**32))]))
            recs.append(Rec(OP_BMSOLID, [n, mode | opac << 8, 0, n % 4 == 0, argb],
                            [(bytes(4 * n), (al + 3) % 16), (px_bytes(lhs), PAGE_END),
                             (cov.tobytes(), al)], f"simd solid {BM_NAMES[mode]} n={n}"))
    for n in sizes:
        al = int(rng.integers(0, 16))
        p, q = mixed_px(rng, n), mixed_px(rng, n)
        m = mixed_cov(rng, n)
        recs.append(Rec(OP_PREMUL, [n, 0], [(bytes(4 * n), al), (px_bytes(p), (al * 3) % 16)], "simd premul"))
        recs.append(Rec(OP_UNPREMUL, [n, 1], [(bytes(4), 0), (px_bytes(p), al)], "simd unpremul"))
        recs.append(Rec(OP_LERP, [n, n % 3], [(bytes(4 * n), al), (px_bytes(p), (al + 1) % 16),
                                              (px_bytes(q), (al + 2) % 16), (m.tobytes(), PAGE_END)],
                        "simd lerp"))
        recs.append(Rec(OP_ERASE, [n, (al * 37) % 256], [(px_bytes(p), al), (m.tobytes(), (al * 3) % 16)],
                        "simd erase"))
        for op in (OP_VIEW, OP_VIEWPM):
            cfg = struct.pack("<iIII", 1 + n % 9, 0x00C0C0C0, 0x00FFFFFF, 0)
            recs.append(Rec(op, [n, al * 3 - 20, al - 7], [(bytes(4 * n), al), (px_bytes(p), (al * 5) % 16),
                                                           (cfg, 0)], "simd view"))
    sse = FxRunner(run.target, run.out, ["--simd", "sse2"])
    a, b = run.run(recs), sse.run(recs)
    for r, x, y in zip(recs, a, b):
        rep.check(x.res == y.res and x.bufs == y.bufs and x.extra == y.extra, r.name, "auto != sse2")
    # speed-up (best of 3, alternating)
    n, reps = 1 << 20, 10
    lhs, rhs = rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32), \
        rng.integers(0, 2**32, n, dtype=np.uint64).astype(np.uint32)
    cov = rng.integers(0, 256, n).astype(np.uint8)
    brecs, names = [], []
    for mode in modes:
        brecs.append(Rec(OP_BM, [n, mode | 255 << 8, 1, 0, 0, 0, 0, reps],
                         [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 0)]))
        names.append(f"{BM_NAMES[mode]} random")
        brecs.append(Rec(OP_BM, [n, mode | 255 << 8, 1, 0, 0, 0, 0, reps],
                         [(bytes(4), 0), (px_bytes(lhs | 0xFF000000), 0), (px_bytes(rhs | 0xFF000000), 0)]))
        names.append(f"{BM_NAMES[mode]} opaque")
        brecs.append(Rec(OP_BMCOV, [n, mode | 200 << 8, 1, 0, 0, 0, 0, reps],
                         [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 0), (cov.tobytes(), 0)]))
        names.append(f"{BM_NAMES[mode]} cov+opacity")
    cfg = struct.pack("<iIII", 8, 0xFFFFFF, 0xCCCCCC, 0)
    for op, nm in ((OP_PREMUL, "k_premul_row"), (OP_UNPREMUL, "k_unpremul_row")):
        brecs.append(Rec(op, [n, 1, 0, 0, 0, 0, 0, reps], [(bytes(4), 0), (px_bytes(lhs), 0)]))
        names.append(nm)
    brecs.append(Rec(OP_LERP, [n, 1, 0, 0, 0, 0, 0, reps],
                     [(bytes(4), 0), (px_bytes(lhs), 0), (px_bytes(rhs), 0), (cov.tobytes(), 0)]))
    names.append("k_lerp_mask_row")
    for op, nm in ((OP_VIEW, "k_view_row"), (OP_VIEWPM, "k_view_row_pm")):
        brecs.append(Rec(op, [n, 0, 0, 0, 0, 0, 0, reps], [(bytes(4 * n), 0), (px_bytes(lhs), 0), (cfg, 0)]))
        names.append(nm)
    ta, tb = [10**18] * len(brecs), [10**18] * len(brecs)
    for _ in range(3):
        for t, rr in ((tb, sse.run(brecs)), (ta, run.run(brecs))):
            for i, r in enumerate(rr):
                t[i] = min(t[i], r.time_us())
    print(f"[{rep.target}] simd speed-up (1 Mpx x {reps}, best of 3): sse2 / default Mpx/s")
    for nm, x, y in zip(names, tb, ta):
        print(f"    {nm:<24} {n * reps / max(x, 1):7.0f} {n * reps / max(y, 1):7.0f}  x{x / max(y, 1):.2f}")
    rep.done("simd", t0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/g1")
    ap.add_argument("--only", default="", help="comma list of suites: straight,bm,sizes,bench,simd")
    ap.add_argument("--modes", default="", help="comma list of mode numbers (default all)")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--no-bench", action="store_true")
    ap.add_argument("--seed", type=int, default=1234)
    ap.add_argument("--simd", default="", help="pass --simd VALUE to fxtest (e.g. sse2)")
    args = ap.parse_args()
    out = (ROOT / args.out).resolve()
    targets = ["win", "linux"] if args.target == "both" else [args.target]
    only = set(filter(None, args.only.split(",")))
    modes = [int(m) for m in args.modes.split(",")] if args.modes else list(range(15))
    fails = 0
    for t in targets:
        if not args.no_build:
            build.build_app("fxtest", t, out)
        run = FxRunner(t, out, ["--simd", args.simd] if args.simd else [])
        rng = np.random.default_rng(args.seed)
        suites = [("straight", lambda r: suite_straight(run, r, rng)),
                  ("bm", lambda r: suite_bm(run, r, rng, modes)),
                  ("sizes", lambda r: suite_sizes(run, r, rng, modes))]
        if not args.no_bench:
            suites.append(("bench", lambda r: suite_bench(run, r, rng)))
        suites.append(("simd", lambda r: suite_simd(run, r, rng, modes)))
        for name, fn in suites:
            if not only or name in only:
                rep = Report(t)
                fn(rep)
                fails += rep.fails
    print("blend_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
