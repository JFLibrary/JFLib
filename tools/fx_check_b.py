#!/usr/bin/env python3
"""fx_check_b.py -- checks of the FXB effect kernels (ASMLIB/src/fx: fx_blur, fx_artistic,
fx_noise, fx_photo, fx_object, helpers fx_conv / fx_lochist) through apps/efftest.

  python tools/fx_check_b.py [--target win|linux|both] [--out build/fxb] [--no-build]
                             [--only NAME,...] [--bench] [--shots DIR] [--size WxH]

Every effect has a numpy reference with the documented semantics:
bit-exact for the integer pipelines, |d| <= 1..2 where float rounding differs.  Besides the
references: tile / thread determinism (tiles 64 / 256 / whole, threads 1 / N identical),
cancel, extreme parameters (radius 0 = copy, max radius on small images, 1 x 1 images),
transparent pixels (no colour bleeding from alpha-0 neighbours).  --shots DIR writes PNGs of
every effect on a generated natural-looking test image for visual inspection; --bench times
4000 x 3000 runs with the default thread pool.
"""

import argparse
import math
import os
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402
import fx_check as fc  # noqa: E402  (FXA's harness: Harness, Report, generic_checks, images)

RAW_MAGIC = 0x31525352
FAILS = []
NCHECK = [0]


# ================================================================== image I/O

def write_raw(path, bgra):
    h, w = bgra.shape[:2]
    with open(path, "wb") as f:
        f.write(struct.pack("<4I", RAW_MAGIC, w, h, 3))
        f.write(np.ascontiguousarray(bgra, dtype=np.uint8).tobytes())


def read_raw(path):
    b = Path(path).read_bytes()
    magic, w, h, fmt = struct.unpack_from("<4I", b, 0)
    assert magic == RAW_MAGIC, path
    return np.frombuffer(b, np.uint8, w * h * 4, 16).reshape(h, w, 4).copy()


def write_png(path, bgra, checker=True):
    """PNG of a straight BGRA image; alpha composited over a checkerboard when checker."""
    h, w = bgra.shape[:2]
    rgb = bgra[..., 2::-1].astype(np.float64)
    if checker:
        a = bgra[..., 3:4].astype(np.float64) / 255.0
        yy, xx = np.mgrid[0:h, 0:w]
        bg = np.where(((yy // 8) + (xx // 8)) % 2 == 0, 255.0, 204.0)[..., None]
        rgb = rgb * a + bg * (1 - a)
    rgb = np.clip(np.rint(rgb), 0, 255).astype(np.uint8)
    raw = b"".join(b"\0" + rgb[y].tobytes() for y in range(h))

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
    Path(path).write_bytes(png)


# ================================================================== test images

def value_noise(h, w, cell, rng):
    gh, gw = h // cell + 3, w // cell + 3
    g = rng.random((gh, gw))
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    fy, fx = yy / cell, xx / cell
    iy, ix = fy.astype(int), fx.astype(int)
    ty, tx = fy - iy, fx - ix
    ty, tx = ty * ty * (3 - 2 * ty), tx * tx * (3 - 2 * tx)
    a = g[iy, ix] * (1 - tx) + g[iy, ix + 1] * tx
    b = g[iy + 1, ix] * (1 - tx) + g[iy + 1, ix + 1] * tx
    return a * (1 - ty) + b * ty


def natural(w, h, seed=1, alpha=True):
    """A photo-like test image: sky gradient, sun, textured hills, soft shapes, fine lines,
    bright highlights, a red-eye pair, a semi-transparent strip (straight BGRA)."""
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    u, v = xx / max(w - 1, 1), yy / max(h - 1, 1)
    img = np.zeros((h, w, 3))
    sky_top, sky_bot = np.array([200, 120, 40]), np.array([120, 190, 250])   # BGR
    img[:] = sky_top * (1 - v[..., None]) + sky_bot * v[..., None]
    # sun
    d = np.hypot(xx - 0.75 * w, yy - 0.25 * h)
    sun = np.clip((0.09 * min(w, h) - d) / 2.0, 0, 1)[..., None]
    img = img * (1 - sun) + np.array([150, 240, 255]) * sun
    # hills with texture
    tex = sum(value_noise(h, w, c, rng) / (i + 1) for i, c in enumerate((64, 16, 4)))
    tex = (tex - tex.min()) / (np.ptp(tex) + 1e-9)
    hill = 0.55 * h + 0.08 * h * np.sin(xx / w * 7.0) + 0.05 * h * np.sin(xx / w * 17.0 + 1)
    m = np.clip((yy - hill) / 1.5, 0, 1)[..., None]
    grass = np.stack([30 + 40 * tex, 90 + 110 * tex, 40 + 60 * tex], -1)
    img = img * (1 - m) + grass * m
    # soft coloured shapes
    for (cx, cy, r, col) in ((0.2, 0.7, 0.10, (40, 40, 220)), (0.5, 0.8, 0.07, (220, 60, 60)),
                             (0.35, 0.45, 0.05, (60, 200, 230))):
        d = np.hypot(xx - cx * w, yy - cy * h)
        a = np.clip((r * min(w, h) - d) / 1.2, 0, 1)[..., None]
        img = img * (1 - a) + np.array(col) * a
    # a rectangle with hard edges and fine lines
    x0, x1, y0, y1 = int(0.05 * w), int(0.3 * w), int(0.1 * h), int(0.35 * h)
    img[y0:y1, x0:x1] = (235, 235, 235)
    for k in range(y0 + 3, y1 - 2, 4):
        img[k, x0 + 3:x1 - 3] = (20, 20, 20)
    # small highlights (bokeh) and a red-eye pair
    for i in range(24):
        cx, cy = rng.integers(0, w), rng.integers(0, int(0.5 * h) + 1)
        img[max(cy - 1, 0):cy + 1, max(cx - 1, 0):cx + 1] = (255, 255, 255)
    for ex in (0.55, 0.62):
        cx, cy = ex * w, 0.62 * h
        d = np.hypot(xx - cx, yy - cy)
        rr = 0.025 * min(w, h) + 1
        iris = np.clip((rr - d) / 1.0, 0, 1)[..., None]
        img = img * (1 - iris) + np.array([30, 30, 200]) * iris
        pup = np.clip((0.35 * rr - d) / 1.0, 0, 1)[..., None]
        img = img * (1 - pup) + np.array([10, 10, 90]) * pup
    noise = rng.normal(0, 3.0, img.shape)
    img = np.clip(np.rint(img + noise), 0, 255)
    a = np.full((h, w), 255.0)
    if alpha:
        a = np.where(u > 0.88, np.clip(255 * (1 - (u - 0.88) / 0.1), 0, 255), 255.0)
        a[:, int(0.97 * w):] = 0
    out = np.zeros((h, w, 4), np.uint8)
    out[..., :3] = img.astype(np.uint8)
    out[..., 3] = np.rint(a).astype(np.uint8)
    # transparent pixels carry garbage colour (must not bleed)
    t = out[..., 3] == 0
    out[t, :3] = rng.integers(0, 256, (int(t.sum()), 3), dtype=np.uint8)
    return out


def rand_img(rng, w, h, alpha="mixed"):
    img = rng.integers(0, 256, (h, w, 4), dtype=np.uint8)
    if alpha == "opaque":
        img[..., 3] = 255
    elif alpha == "mixed":
        sel = rng.random((h, w))
        img[..., 3] = np.where(sel < 0.4, 255, np.where(sel < 0.55, 0, img[..., 3]))
    return img


# ================================================================== runner

class Runner:
    def __init__(self, target, out):
        self.target, self.out = target, Path(out)
        self.exe = build.exe_path("efftest", target, self.out)
        self.tmp = self.out / "fxb_tmp" / target
        self.tmp.mkdir(parents=True, exist_ok=True)
        self.n = 0

    def run(self, args, timeout=900):
        cmd = build.run_cmd_for(self.exe, self.target, ["--shot", "-"] + [str(a) for a in args])
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr

    def path(self, name):
        p = self.tmp / name
        if self.target == "linux":
            return build.to_wsl_path(p)
        return str(p)

    def fx(self, name, src, params=None, tile=256, threads=1, roi=None, extra=(), fill=None):
        """run effect NAME on src (h, w, 4 straight BGRA) -> (result, info line)"""
        self.n += 1
        fin, fout = f"in_{self.n % 4}.raw", f"out_{self.n % 4}.raw"
        write_raw(self.tmp / fin, src)
        if (self.tmp / fout).exists():
            (self.tmp / fout).unlink()
        args = ["--fx", name, "--in", self.path(fin), "--out", self.path(fout),
                "--tile", tile, "--threads", threads]
        for k, v in (params or {}).items():
            args += ["--p", f"{k}={v}"]
        if roi:
            args += ["--roi", ",".join(map(str, roi))]
        if fill is not None:
            args += ["--fill", "%08X" % fill]
        args += list(extra)
        rc, out = self.run(args)
        line = next((ln for ln in out.splitlines() if ln.startswith(("ok ", "err "))), out.strip())
        if rc != 0 or not line.startswith("ok"):
            return None, line
        return read_raw(self.tmp / fout), line


def check(cond, what):
    NCHECK[0] += 1
    if not cond:
        FAILS.append(what)
        print("  FAIL", what)
    return cond


def compare(name, got, ref, tol=0, mask=None):
    if got is None:
        return check(False, f"{name}: no result")
    d = np.abs(got.astype(np.int32) - ref.astype(np.int32))
    if mask is not None:
        d = d * mask[..., None]
    m = int(d.max()) if d.size else 0
    ok = m <= tol
    if not ok:
        idx = np.unravel_index(np.argmax(d), d.shape)
        print(f"  {name}: max|d|={m} at y={idx[0]} x={idx[1]} c={idx[2]} "
              f"got={got[idx[0], idx[1]]} ref={ref[idx[0], idx[1]]} "
              f"n>tol={int((d > tol).sum())}")
    return check(ok, f"{name}: max|d|={m} > {tol}")


# ================================================================== library pipelines (exact)

def to_work(img):
    """rs_to_work, FMT_ARGB, no gamma -> int64 (h, w, 4) premultiplied, full = 16320"""
    c = img[..., :3].astype(np.int64)
    a = img[..., 3:4].astype(np.int64)
    p = np.empty(img.shape, np.int64)
    p[..., :3] = ((c * a + 2) * 16448) >> 16
    p[..., 3] = a[..., 0] << 6
    return p


def from_work(p):
    """rs_from_work, FMT_ARGB, no gamma"""
    p = np.clip(p, -32768, 32767)
    A = np.maximum(p[..., 3:4], 0)
    c = np.maximum(np.minimum(p[..., :3], A), 0)
    a8 = (np.minimum(A, 16320) + 32) >> 6
    cf = (c.astype(np.float32) * np.float32(255)) / np.maximum(A, 1).astype(np.float32)
    c8 = np.clip(np.rint(cf), 0, 255)
    out = np.empty(p.shape, np.uint8)
    out[..., :3] = c8.astype(np.uint8)
    out[..., 3] = np.clip(a8[..., 0], 0, 255).astype(np.uint8)
    return out


def kern_gauss(var12):
    """fxc_kern_gauss -> ("fir", weights) | ("box", [h1, h2, h3]) | ("copy",)"""
    if var12 <= 0:
        return ("copy",)
    if var12 <= 432:
        h = math.sqrt(var12 * 0.75)
        hi = int(h)
        if h > hi:
            hi += 1
        h = min(max(hi, 1), 18)
        c = -6.0 / var12
        g = [math.exp(c * float(i * i)) for i in range(h + 1)]
        s = g[0]
        for i in range(1, h + 1):
            s += g[i] + g[i]
        k = 16384.0 / s
        q = [int(math.floor(gi * k + 0.5)) for gi in g]
        tot = q[0] + 2 * sum(q[1:])
        q[0] += 16384 - tot
        return ("fir", [q[abs(j - h)] for j in range(2 * h + 1)])
    wl = 1
    while 3 * (wl + 2) ** 2 <= var12 + 3:
        wl += 2
    num = 3 * wl * wl + 12 * wl + 9 - var12
    den = 4 * wl + 4
    m = min((2 * num + den) // (2 * den), 3)
    return ("box", [((wl if k < m else wl + 2) - 1) // 2 for k in range(3)])


def var12(r):
    return 2 * r * (r + 2) if r > 0 else 0


def box_1d(x, h, axis):
    """box means of width 2h+1 along axis over an already padded array (len - 2h outputs)"""
    w = 2 * h + 1
    cs = np.cumsum(x, axis=axis, dtype=np.int64)
    z = np.zeros_like(np.take(cs, [0], axis=axis))
    cs = np.concatenate([z, cs], axis=axis)
    n = x.shape[axis] - 2 * h
    s = np.take(cs, np.arange(w, w + n), axis=axis) - np.take(cs, np.arange(0, n), axis=axis)
    inv = np.float32(1.0) / np.float32(w)
    return np.rint(s.astype(np.float32) * inv).astype(np.int64)


def fir_1d(x, q, axis):
    taps = len(q)
    n = x.shape[axis] - taps + 1
    acc = np.zeros(np.take(x, np.arange(n), axis=axis).shape, np.int64)
    for j, qj in enumerate(q):
        acc += qj * np.take(x, np.arange(j, j + n), axis=axis)
    return (acc + 8192) >> 14


def apply_kern_axis(W, kern, axis):
    """clamp-extend W along axis by the kernel halo, then convolve"""
    if kern[0] == "copy":
        return W
    halo = (len(kern[1]) - 1) // 2 if kern[0] == "fir" else sum(kern[1])
    n = W.shape[axis]
    idx = np.clip(np.arange(-halo, n + halo), 0, n - 1)
    X = np.take(W, idx, axis=axis)
    if kern[0] == "fir":
        return fir_1d(X, kern[1], axis)
    for h in kern[1]:
        X = box_1d(X, h, axis)
    return X


def ref_sep(img, kern, kernv=None):
    W = to_work(img)
    H = apply_kern_axis(W, kern, 1)
    V = apply_kern_axis(H, kernv or kern, 0)
    return from_work(V)


def disc_edges(r):
    cutoff = ((2 * r + 1) ** 2 + 2) // 4
    E = []
    for v in range(r + 1):
        u = 0
        while u < r and (u + 1) ** 2 + v * v <= cutoff:
            u += 1
        E.append(u)
    return E


def ref_disc_sum(V, r):
    """sum over the paint.net disc of every pixel, clamped edges (V: h, w, c int64)"""
    h, w = V.shape[:2]
    E = disc_edges(r)
    Vx = V[:, np.clip(np.arange(-r, w + r), 0, w - 1)]
    P = np.concatenate([np.zeros((h, 1) + V.shape[2:], np.int64), np.cumsum(Vx, 1)], 1)
    acc = np.zeros(V.shape, np.int64)
    for v in range(-r, r + 1):
        e = E[abs(v)]
        rows = np.clip(np.arange(h) + v, 0, h - 1)
        Pr = P[rows]
        acc += Pr[:, r + e + 1: r + e + 1 + w] - Pr[:, r - e: r - e + w]
    area = sum(2 * E[abs(v)] + 1 for v in range(-r, r + 1))
    return acc, area


def ref_unfocus(img, r):
    acc, area = ref_disc_sum(to_work(img), r)
    inv = np.float32(1.0) / np.float32(area)
    return from_work(np.rint(acc.astype(np.float32) * inv).astype(np.int64))


def window_stack(img, r, shape="disc"):
    """(K, h, w, 4) window values (invalid = -1) and validity (K, h, w) of every pixel"""
    h, w = img.shape[:2]
    E = disc_edges(r) if shape == "disc" else [r] * (r + 1)
    offs = [(v, u) for v in range(-r, r + 1) for u in range(-E[abs(v)], E[abs(v)] + 1)]
    vals = np.empty((len(offs), h, w, 4), np.int32)
    valid = np.empty((len(offs), h, w), bool)
    ys0, xs0 = np.arange(h), np.arange(w)
    for k, (v, u) in enumerate(offs):
        ys, xs = ys0 + v, xs0 + u
        m = ((ys >= 0) & (ys < h))[:, None] & ((xs >= 0) & (xs < w))[None, :]
        sub = img[np.clip(ys, 0, h - 1)][:, np.clip(xs, 0, w - 1)].astype(np.int32)
        valid[k] = m
        vals[k] = np.where(m[..., None], sub, -1)
    return vals, valid


def kth(vals, mask, t):
    """t-th smallest (1-based, per pixel) of vals where mask; big where not enough"""
    s = np.sort(np.where(mask, vals, 1 << 20), axis=0)
    t = np.clip(t, 1, s.shape[0])
    return np.take_along_axis(s, (t - 1)[None], 0)[0]


def ref_percentiles(img, r, pct, vals=None, valid=None):
    if vals is None:
        vals, valid = window_stack(img, r)
    nall = valid.sum(0)
    colv = valid & (vals[..., 3] > 0)
    ncol = colv.sum(0)
    out = np.zeros(img.shape, np.int32)
    out[..., 3] = np.minimum(kth(vals[..., 3], valid, np.maximum(nall * pct // 100, 1)), 255)
    tc = np.maximum(ncol * pct // 100, 1)
    for c in range(3):
        out[..., c] = np.where(ncol > 0, np.minimum(kth(vals[..., c], colv, tc), 255), 0)
    return out


def ref_median(img, r, pct):
    return ref_percentiles(img, r, pct).astype(np.uint8)


def sincos(x):
    import xform_check as xc
    return xc.sincos(x)


def ref_motion(img, angle, d, centered):
    th = (angle + 180.0) * 0.017453292519943295
    s, c = sincos(th)
    dd = float(d)
    ex, ey = c * dd, -(s * dd)
    sx = sy = 0.0
    if centered:
        ex, ey = ex * 0.5, ey * 0.5
        sx, sy = -ex, -ey
    n = ((1 + d) * 3) // 2
    pts = []
    for j in range(n):
        t = 0.0 if n == 1 else float(j) / float(n - 1)
        pts.append((round(((ex - sx) * t + sx) * 16.0), round(((ey - sy) * t + sy) * 16.0)))
    W = to_work(img)
    h, w = img.shape[:2]
    yy, xx = np.mgrid[0:h, 0:w]
    acc = np.zeros(W.shape, np.int64)
    cnt = np.zeros((h, w), np.int64)
    for Px, Py in pts:
        X, Y = 16 * xx + Px, 16 * yy + Py
        ok = (X >= 0) & (X <= 16 * (w - 1)) & (Y >= 0) & (Y <= 16 * (h - 1))
        ix, fx, iy, fy = X >> 4, X & 15, Y >> 4, Y & 15
        x0, x1 = np.clip(ix, 0, w - 1), np.clip(ix + 1, 0, w - 1)
        y0, y1 = np.clip(iy, 0, h - 1), np.clip(iy + 1, 0, h - 1)
        v = ((16 - fx) * (16 - fy))[..., None] * W[y0, x0] + (fx * (16 - fy))[..., None] * W[y0, x1]             + ((16 - fx) * fy)[..., None] * W[y1, x0] + (fx * fy)[..., None] * W[y1, x1]
        acc += np.where(ok[..., None], v, 0)
        cnt += ok
    inv = np.float32(1.0) / np.maximum(256 * cnt, 1).astype(np.float32)
    res = np.rint(acc.astype(np.float32) * inv[..., None]).astype(np.int64)
    res = np.where((cnt > 0)[..., None], res, W)
    return from_work(res)


def premul_vec(p):
    """{B*A, G*A, R*A, A} int64 of straight pixels"""
    p = p.astype(np.int64)
    out = p * p[..., 3:4]
    out[..., 3] = p[..., 3]
    return out


def blend_out(acc, n):
    """paint.net ColorBgra.Blend of alpha-weighted sums"""
    sA = acc[..., 3]
    out = np.zeros(acc.shape, np.int64)
    ok = (n > 0) & (sA > 0)
    sAs = np.where(ok, sA, 1)
    for c in range(3):
        out[..., c] = np.where(ok, acc[..., c] // sAs, 0)
    out[..., 3] = np.where(ok, sA // np.maximum(n, 1), 0)
    return np.clip(out, 0, 255).astype(np.uint8)


def ref_zoom(img, amount, ox, oy):
    h, w = img.shape[:2]
    fcx = int(w * ox * 32768.0) + (w << 15)
    fcy = int(h * oy * 32768.0) + (h << 15)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.int64)
    fx, fy = (xx << 16) - fcx, (yy << 16) - fcy
    acc = premul_vec(img)
    n = np.ones((h, w), np.int64)
    for _ in range(64):
        fx = fx - (((fx >> 4) * amount) >> 10)
        fy = fy - (((fy >> 4) * amount) >> 10)
        u, v = (fx + fcx + 32768) >> 16, (fy + fcy + 32768) >> 16
        ok = (u >= 0) & (u < w) & (v >= 0) & (v < h)
        acc += np.where(ok[..., None], premul_vec(img[np.clip(v, 0, h - 1), np.clip(u, 0, w - 1)]), 0)
        n += ok
    return blend_out(acc, n)


def tdiv(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b > 0) else -q


def ref_radial(img, angle, ox, oy, q):
    h, w = img.shape[:2]
    n = q * q * (30 + q * q)
    fr = int(angle * math.pi * 65536.0 / 181.0)
    fsr = tdiv(fr, n)
    r2 = (fsr * fsr) >> 11
    fcx = (w << 15) + int(ox * float(w << 15))
    fcy = (h << 15) + int(oy * float(h << 15))
    yy, xx = np.mgrid[0:h, 0:w].astype(np.int64)
    x1 = x2 = (xx << 16) - fcx
    y1 = y2 = (yy << 16) - fcy
    acc = premul_vec(img)
    cnt = np.ones((h, w), np.int64)

    def rot(x, y, r):
        return (x - (((y >> 8) * r) >> 8) - (((x >> 14) * r2) >> 8),
                y + (((x >> 8) * r) >> 8) - (((y >> 14) * r2) >> 8))
    for _ in range(n):
        x1, y1 = rot(x1, y1, fsr)
        x2, y2 = rot(x2, y2, -fsr)
        for (x, y) in ((x1, y1), (x2, y2)):
            u, v = (x + fcx + 32768) >> 16, (y + fcy + 32768) >> 16
            ok = (u >= 0) & (u < w) & (v >= 0) & (v < h)
            acc += np.where(ok[..., None], premul_vec(img[np.clip(v, 0, h - 1), np.clip(u, 0, w - 1)]), 0)
            cnt += ok
    return blend_out(acc, cnt)


def round_away(x):
    return int(math.floor(abs(x) + 0.5)) * (1 if x >= 0 else -1)


def ref_fragment(img, n, d, rot):
    h, w = img.shape[:2]
    t0 = (rot - 90.0) * math.pi / 180.0
    step = 2.0 * math.pi / n
    offs = []
    for i in range(n):
        s, c = sincos(float(i) * step + t0)
        offs.append((round_away(-s * float(d)), round_away(-c * float(d))))
    yy, xx = np.mgrid[0:h, 0:w]
    acc = np.zeros(img.shape, np.int64)
    cnt = np.zeros((h, w), np.int64)
    for ox, oy in offs:
        u, v = xx - ox, yy - oy
        ok = (u >= 0) & (u < w) & (v >= 0) & (v < h)
        acc += np.where(ok[..., None], premul_vec(img[np.clip(v, 0, h - 1), np.clip(u, 0, w - 1)]), 0)
        cnt += ok
    return blend_out(acc, cnt)


def _s2l(x):
    return x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4


def _l2s(y):
    return 12.92 * y if y <= 0.0031308 else 1.055 * y ** (1 / 2.4) - 0.055


LIN16 = np.array([math.floor(16320 * _s2l(v / 255) + 0.5) for v in range(256)], np.int64)
SRGB8 = np.array([math.floor(255 * _l2s(i / 4095) + 0.5) for i in range(4096)], np.int64)


def to_work_g(img):
    """rs_to_work, FMT_ARGB, RS_GAMMA"""
    c = img.astype(np.int64)
    a = c[..., 3]
    P = np.zeros(c.shape, np.int64)
    P[..., 3] = a << 6
    P[..., :3] = (LIN16[c[..., :3]] * a[..., None] * 257 + 32768) >> 16
    return P


def from_work_g(P):
    """rs_from_work, FMT_ARGB, RS_GAMMA"""
    P = np.clip(P, -32768, 32767)
    Pr = P[..., 3]
    Pa = np.maximum(Pr, 0)
    Pc = np.maximum(np.minimum(P[..., :3], Pr[..., None]), 0)
    out = np.zeros(P.shape, np.int64)
    out[..., 3] = (np.minimum(Pa, 16320) + 32) >> 6
    pa = np.maximum(Pa, 1)[..., None]
    out[..., :3] = SRGB8[np.minimum((Pc * 8190 + pa) // (2 * pa), 4095)]
    out[Pa == 0] = 0
    return out.astype(np.uint8)


def hash32(x, y, seed, stream):
    """fx_hash32 (fx_common.s)"""
    m = np.uint64(0xFFFFFFFF)

    def u(v):
        return np.asarray(v, np.int64).astype(np.uint64) & m
    a = (u(x) * np.uint64(0x8DA6B343)) & m
    a ^= (u(y) * np.uint64(0xD8163841)) & m
    a ^= (u(seed) * np.uint64(0xCB1AB31F)) & m
    a ^= (u(stream) * np.uint64(0x165667B1)) & m
    a ^= a >> np.uint64(16)
    a = (a * np.uint64(0x85EBCA6B)) & m
    a ^= a >> np.uint64(13)
    a = (a * np.uint64(0xC2B2AE35)) & m
    a ^= a >> np.uint64(16)
    return a.astype(np.int64)


def ref_surface(img, r, t):
    vals, valid = window_stack(img, r)
    c0 = -1.0 / float(2 * t * t)
    WT = np.array([int(math.exp(float(d * d) * c0) * 256.0 + 0.5) for d in range(256)], np.int64)
    colv = valid & (vals[..., 3] > 0)
    ncol = colv.sum(0)
    med = ref_percentiles(img, r, 50, vals, valid)
    centre = img.astype(np.int64).copy()
    tr = img[..., 3] == 0
    centre[tr, :3] = med[tr, :3]
    out = np.zeros(img.shape, np.int64)
    for ch in range(4):
        m = valid if ch == 3 else colv
        v = centre[..., ch]
        wgt = np.where(m, WT[np.clip(np.abs(vals[..., ch] - v[None]), 0, 255)], 0)
        den = wgt.sum(0)
        num = (wgt * vals[..., ch]).sum(0)
        o = np.where(den > 0, (2 * num + den) // np.maximum(2 * den, 1), v)
        if ch < 3:
            o = np.where(ncol > 0, o, 0)
        out[..., ch] = o
    return out.astype(np.uint8)


def ref_bokeh(img, r, inten, gran):
    W = to_work_g(img)
    f32 = np.float32
    K = f32(0.0025 * float(inten * inten))
    th64 = 0.95 - float(gran) * 0.009
    th, inv = f32(th64), f32(1.0 / (1.0 - th64))
    A = W[..., 3]
    lumN = 2126 * W[..., 2] + 7152 * W[..., 1] + 722 * W[..., 0]
    with np.errstate(divide="ignore", invalid="ignore"):
        lum = lumN.astype(f32) / (A * 10000).astype(f32)
    t = np.minimum(np.maximum((lum - th) * inv, f32(0)), f32(1))
    f = np.where(A > 0, f32(1) + (t * t) * K, f32(1)).astype(f32)
    V = W.copy()
    V[..., :3] = np.rint(W[..., :3].astype(f32) * f[..., None]).astype(np.int64)
    h, w = img.shape[:2]
    E = disc_edges(r)
    Vx = V[:, np.clip(np.arange(-r, w + r), 0, w - 1)]
    P = np.concatenate([np.zeros((h, 1, 4), np.int64), np.cumsum(Vx, 1)], 1)
    acc = np.zeros((h, w, 4), f32)
    for v in range(-r, r + 1):
        e = E[abs(v)]
        Pr = P[np.clip(np.arange(h) + v, 0, h - 1)]
        acc = acc + (Pr[:, r + e + 1: r + e + 1 + w] - Pr[:, r - e: r - e + w]).astype(f32)
    area = sum(2 * E[abs(v)] + 1 for v in range(-r, r + 1))
    m = acc * (f32(1.0) / f32(area))
    Am = m[..., 3]
    for c in range(3):
        mc = m[..., c]
        sel = (Am > 0) & (mc > Am * f32(0.8))
        x = mc.astype(np.float64) / np.where(sel, Am, 1).astype(np.float64)
        xp = (1.0 - np.exp((x - 0.8) * -5.0)) * 0.2 + 0.8
        m[..., c] = np.where(sel, (xp * Am.astype(np.float64)).astype(f32), mc)
    return from_work_g(np.rint(m).astype(np.int64))


def ref_sketch(img, R, ang, spread, seed):
    h, w = img.shape[:2]
    W = to_work(img)
    s0, c0 = sincos(ang * 0.017453292519943295)
    L = float(2 * R + 1)
    ou = [float(hash32(k, 0, seed, 7) & 0xFFFF) * 0.0000152587890625 * L for k in range(3)]
    ov = [float(hash32(k, 1, seed, 7) & 0xFFFF) * 0.0000152587890625 * 4.0 for k in range(3)]
    dirs = np.zeros((256, 4), np.int64)
    for k in range(256):
        th = ((float(k) / 255.0 - 0.5) * float(spread) + ang) * 0.017453292519943295
        sn, cs = sincos(th)
        dirs[k] = (round(cs * (float(R) * -256.0)), round(sn * (float(R) * -256.0)),
                   round(cs * 256.0), round(sn * 256.0))
    yy, xx = np.mgrid[0:h, 0:w].astype(np.int64)
    cx, cy = xx + 0.5, yy + 0.5
    u = cx * c0 + cy * s0
    v = cy * c0 - cx * s0
    acc = np.zeros(W.shape, np.int64)
    for layer in range(3):
        m = np.floor((u + ou[layer]) / L).astype(np.int64)
        k = np.floor((v + ov[layer]) * 0.25).astype(np.int64)
        d = dirs[hash32(m, k, seed, layer + 1) & 255]
        Px, Py = 256 * xx + d[..., 0], 256 * yy + d[..., 1]
        for _ in range(2 * R + 1):
            X = np.clip(Px >> 4, 0, 16 * (w - 1))
            Y = np.clip(Py >> 4, 0, 16 * (h - 1))
            ix, fx, iy, fy = X >> 4, X & 15, Y >> 4, Y & 15
            x1, y1 = np.minimum(ix + 1, w - 1), np.minimum(iy + 1, h - 1)
            acc += (((16 - fx) * (16 - fy))[..., None] * W[iy, ix]
                    + (fx * (16 - fy))[..., None] * W[iy, x1]
                    + ((16 - fx) * fy)[..., None] * W[y1, ix]
                    + (fx * fy)[..., None] * W[y1, x1])
            Px, Py = Px + d[..., 2], Py + d[..., 3]
    inv = np.float32(1.0) / np.float32(256 * 3 * (2 * R + 1))
    return from_work(np.rint(acc.astype(np.float32) * inv).astype(np.int64))

def u32(img):
    return np.ascontiguousarray(img, np.uint8).view(np.uint32)[..., 0]


def u8(p):
    p = np.ascontiguousarray(np.asarray(p, np.uint32))
    return p[..., None].view(np.uint8).reshape(p.shape + (4,)).copy()


def blend(lhs, rhs, mode, opacity=255):
    import blend_check as bc
    return u8(bc.ref_blend(u32(lhs).ravel(), u32(rhs).ravel(), mode, opacity).reshape(lhs.shape[:2]))


def intensity_byte(img):
    c = img.astype(np.int64)
    return (7471 * c[..., 0] + 38470 * c[..., 1] + 19595 * c[..., 2]) >> 16


def bac_table(b, c):
    if c < 0:
        mul, div = c + 100, 100
    elif c > 0:
        mul, div = 100, 100 - c
    else:
        mul, div = 1, 1
    if div == 0:
        return 2, np.array([0 if i + b < 128 else 255 for i in range(256)], np.int64)
    mode = 0 if div == 100 else 1
    tab = [tdiv((i - 127) * mul, div) + 127 - i + b if mode == 0 else
           tdiv((i - 127 + b) * mul, div) + 127 - i for i in range(256)]
    return mode, np.array(tab, np.int64)


def ref_bac(img, b, c):
    mode, tab = bac_table(b, c)
    sh = tab[intensity_byte(img)]
    out = img.copy()
    if mode == 2:
        out[..., :3] = sh[..., None]
    else:
        out[..., :3] = np.clip(img[..., :3].astype(np.int64) + sh[..., None], 0, 255)
    return out


def ref_blurbac(img, r, b, c):
    return ref_bac(ref_sep(img, kern_gauss(var12(r))), b, c)


def ref_glow(img, r, b, c):
    return blend(img, ref_blurbac(img, r, b, c), 12)


def ref_soften(img, soft, light, warm):
    G = ref_blurbac(img, 3 * soft, light, tdiv(-light, 2))
    f32 = np.float32
    wf = f32(warm) / f32(100)
    red, blue = f32(1) + wf, f32(1) - wf
    I = intensity_byte(img)
    L = img.copy()
    L[..., 1] = I
    L[..., 2] = np.clip(np.trunc(I.astype(f32) * red), 0, 255)
    L[..., 0] = np.clip(np.trunc(I.astype(f32) * blue), 0, 255)
    out = blend(L, G, 7)
    out[..., 3] = img[..., 3]
    return out


def ref_sharpen(img, amount):
    m = ref_percentiles(img, amount, 50)
    s = img.astype(np.int64)
    return np.clip((3 * s - m) >> 1, 0, 255).astype(np.uint8)


def ref_redeye(img, tol, sat):
    c = img.astype(np.int64)
    B, G, Rr = c[..., 0], c[..., 1], c[..., 2]
    r, g, b = Rr / 255.0, G / 255.0, B / 255.0
    mn = np.minimum(np.minimum(r, g), b)
    mx = np.maximum(np.maximum(r, g), b)
    delta = mx - mn
    with np.errstate(divide="ignore", invalid="ignore"):
        s = np.where((mx == 0) | (delta == 0), 0, np.trunc(delta / mx * 255.0)).astype(np.int64)
    d = Rr - np.maximum(B, G)
    cond = (d > tol) & (s > 100)
    i = ((0.114 * B + 0.587 * G) + 0.299 * Rr) / 255.0 * 255.0 * (sat / 100.0)
    out = img.copy()
    out[..., 2] = np.where(cond, np.trunc(i).astype(np.int64) & 255, Rr)
    return out


def ref_vignette(img, ox, oy, radius, amount, color):
    h, w = img.shape[:2]
    f32 = np.float32
    cx = (ox + 1.0) * (w * 0.5)
    cy = (oy + 1.0) * (h * 0.5)
    D = math.sqrt((w * 0.5) * (w * 0.5) + (h * 0.5) * (h * 0.5))
    rad = min(max(radius, 0.1), 4.0)
    inv = 1.0 / ((rad + rad) * D)
    amt = min(max(amount, 0.0), 1.0) * float(color >> 24) / 255.0
    colpx = np.array([[[color & 255, (color >> 8) & 255, (color >> 16) & 255, 255]]], np.uint8)
    C = to_work_g(colpx)[0, 0]
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    dy = (yy + 0.5) - cy
    dx = (xx + 0.5) - cx
    rho = np.sqrt(dx * dx + dy * dy) * inv
    t = np.minimum(np.maximum((rho - 0.4) / 0.7, 0.0), 1.0)
    v = ((t * t) * (3.0 - (t + t)) * amt).astype(f32)
    P = to_work_g(img)
    Q = P.copy()
    for ch in range(3):
        Q[..., ch] = (C[ch] * P[..., 3] + 8160) // 16320
    Pn = np.rint(P.astype(f32) + (Q - P).astype(f32) * v[..., None]).astype(np.int64)
    return from_work_g(Pn)


def ref_straighten(img, angle, zoom):
    import xform_check as xc
    h, w = img.shape[:2]
    rad = min(max(angle, -45.0), 45.0) * 0.017453292519943295
    s, c = xc.sincos(abs(rad))
    inv_s = 1.0
    if zoom:
        k = max(float(w) / float(h), float(h) / float(w))
        inv_s = 1.0 / (k * s + c)
    m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
    m = xc.translate(m, w * 0.5, h * 0.5)
    m = xc.rotate(m, -rad, 0.0, 0.0)
    m = xc.scale(m, inv_s, inv_s)
    m = xc.translate(m, -(w * 0.5), -(h * 0.5))
    flags = (1 if zoom else 0) << 8
    out = xc.ref_affine(u32(img), 3, w, h, 3, 3, flags, m)
    return u8(out)

def ref_oil(img, b, L):
    vals, valid = window_stack(img, b, "square")
    c = np.maximum(vals, 0).astype(np.int64)
    c = np.where((c[..., 3] == 0)[..., None], 0, c)
    I = (7471 * c[..., 0] + 38470 * c[..., 1] + 19595 * c[..., 2]) >> 16
    t = I * L + 128
    bucket = ((t >> 8) + t) >> 8
    h, w = img.shape[:2]
    counts = np.zeros((256, h, w), np.int64)
    K = vals.shape[0]
    yy, xx = np.broadcast_to(np.mgrid[0:h, 0:w][:, None], (2, K, h, w))
    np.add.at(counts, (bucket[valid], yy[valid], xx[valid]), 1)
    best = np.argmax(counts, 0)
    n = np.take_along_axis(counts, best[None], 0)[0]
    m = valid & (bucket == best[None])
    A = c[..., 3]
    sA = (A * m).sum(0)
    out = np.zeros(img.shape, np.int64)
    ok = (n > 0) & (sA > 0)
    for ch in range(3):
        out[..., ch] = np.where(ok, (c[..., ch] * A * m).sum(0) // np.maximum(sA, 1), 0)
    out[..., 3] = np.where(ok, sA // np.maximum(n, 1), 0)
    return out.astype(np.uint8)


def desat(img):
    out = img.copy()
    out[..., :3] = intensity_byte(img)[..., None]
    return out


def ref_pencil(img, tip, rng_):
    G = ref_blurbac(img, tip, rng_, -rng_)
    G[..., :3] = 255 - G[..., :3]
    return blend(desat(img), desat(G), 4)


def ref_ink(img, k, col):
    br = -(col - 50) * 2
    glow = blend(img, ref_blurbac(img, 6, br, br), 12).astype(np.int64)
    h, w = img.shape[:2]
    c = img.astype(np.int64)
    c = ((c * c[..., 3:4] + 128) * 257) >> 16
    pad = c[np.clip(np.arange(-2, h + 2), 0, h - 1)][:, np.clip(np.arange(-2, w + 2), 0, w - 1)]
    s = 25 * c
    for dy in range(5):
        for dx in range(5):
            s = s - pad[dy:dy + h, dx:dx + w]
    e = np.clip(s, 0, 255)
    I = (7471 * e[..., 0] + 38470 * e[..., 1] + 19595 * e[..., 2]) >> 16
    d = np.minimum(255, (I * (k + 5)) >> 6)
    out = glow.copy()
    for ch in range(3):
        out[..., ch] = ((glow[..., ch] * (255 - d) + 128) * 257) >> 16
    return out.astype(np.uint8)


def suite_oil(R, a):
    rng = np.random.default_rng(22)
    img = natural(50, 37, 21)
    for b, L in ((3, 50), (1, 3), (5, 255), (8, 20)):
        got, _ = R.fx("oil_painting", img, {"brush_size": b, "coarseness": L}, tile=32)
        compare(f"oil b={b} L={L}", got, ref_oil(img, b, L))
    small = rand_img(rng, 3, 4)
    got, _ = R.fx("oil_painting", small, {"brush_size": 8})
    compare("oil small", got, ref_oil(small, 8, 50))


def suite_pencil(R, a):
    rng = np.random.default_rng(23)
    img = natural(62, 45, 22)
    for t, c in ((2, 0), (1, -20), (20, 20), (7, 13)):
        got, _ = R.fx("pencil_sketch", img, {"pencil_tip_size": t, "color_range": c}, tile=64)
        compare(f"pencil t={t} c={c}", got, ref_pencil(img, t, c))
    small = rand_img(rng, 2, 1)
    got, _ = R.fx("pencil_sketch", small, {"pencil_tip_size": 20})
    compare("pencil small", got, ref_pencil(small, 20, 0))


def suite_ink(R, a):
    rng = np.random.default_rng(24)
    img = natural(66, 47, 23)
    for k, c in ((50, 50), (0, 0), (99, 100), (25, 80)):
        got, _ = R.fx("ink_sketch", img, {"ink_outline": k, "coloring": c}, tile=64)
        compare(f"ink k={k} c={c}", got, ref_ink(img, k, c))
    small = rand_img(rng, 3, 2)
    got, _ = R.fx("ink_sketch", small, {})
    compare("ink small", got, ref_ink(small, 50, 50))

def ref_addnoise(img, inten, sat, cov, seed):
    h, w = img.shape[:2]
    yy, xx = np.mgrid[0:h, 0:w]
    dev = inten * inten // 4
    satq = sat * 4096 // 100
    change = (hash32(xx, yy, seed, 0) % 100) < cov
    n = []
    for k in (1, 2, 3):
        hk = hash32(xx, yy, seed, k)
        s = (hk & 255) + ((hk >> 8) & 255) + ((hk >> 16) & 255) + ((hk >> 24) & 255)
        n.append(((s - 510) * 1774) >> 8)
    r, g, b = n
    i = (4899 * r + 9618 * g + 1867 * b) >> 14
    r, g, b = [i + (((v - i) * satq) >> 12) for v in (r, g, b)]
    out = img.copy()
    for ch, v in ((2, r), (1, g), (0, b)):
        nv = np.clip(img[..., ch].astype(np.int64) + ((v * dev + 32768) >> 16), 0, 255)
        out[..., ch] = np.where(change, nv, img[..., ch])
    return out


def ref_reducenoise(img, r, strength):
    vals, valid = window_stack(img, r)
    colv = valid & (vals[..., 3] > 0)
    ncol = colv.sum(0)
    c = img.astype(np.int64)
    I = ((c[..., 0] * 0.114 + c[..., 1] * 0.587) + c[..., 2] * 0.299) / 255.0
    st = min(max(strength, 0.0), 1.0)
    f = (1.0 - I * 0.75) * st
    out = img.copy()
    for ch in range(3):
        v = c[..., ch]
        n = np.minimum(kth(vals[..., ch], colv, np.maximum(ncol >> 1, 1)), 255)
        val = (n - v).astype(np.float64) * f + v.astype(np.float64)
        out[..., ch] = np.trunc(np.clip(val, 0.0, 255.0)).astype(np.int64)
    keep = (c[..., 3] == 0) | (ncol == 0)
    out[keep] = img[keep]
    return out


def suite_addnoise(R, a):
    img = natural(83, 61, 25)
    for i, s, c, seed in ((64, 100, 100, 0), (0, 100, 100, 1), (100, 400, 50, 99), (37, 0, 100, 1234567),
                          (100, 250, 0, 5)):
        got, _ = R.fx("add_noise", img, {"intensity": i, "color_saturation": s, "coverage": c,
                                         "seed": seed}, tile=64)
        compare(f"addnoise i={i} s={s} c={c} seed={seed}", got, ref_addnoise(img, i, s, c, seed))


def suite_reducenoise(R, a):
    rng = np.random.default_rng(26)
    img = natural(58, 42, 26)
    for r, st in ((6, 0.4), (0, 0.4), (1, 1.0), (12, 0.0), (4, 0.77)):
        got, _ = R.fx("reduce_noise", img, {"radius": r, "strength": st}, tile=64)
        compare(f"reducenoise r={r} s={st}", got, ref_reducenoise(img, r, st))
    small = rand_img(rng, 3, 3)
    got, _ = R.fx("reduce_noise", small, {"radius": 200})
    compare("reducenoise small", got, ref_reducenoise(small, 5, 0.4))

def ref_shadow(img, color, ang, d, r, op, only):
    s, c = sincos(ang * 0.017453292519943295)
    ox, oy = round(c * float(d)), round(s * float(d))
    h, w = img.shape[:2]
    kern = kern_gauss(var12(r))
    H = 0 if kern[0] == "copy" else ((len(kern[1]) - 1) // 2 if kern[0] == "fir" else sum(kern[1]))
    A = np.zeros((h + 2 * H, w + 2 * H), np.int64)
    ys = np.arange(h + 2 * H) - H - oy
    xs = np.arange(w + 2 * H) - H - ox
    my = (ys >= 0) & (ys < h)
    mx = (xs >= 0) & (xs < w)
    sub = img[np.clip(ys, 0, h - 1)][:, np.clip(xs, 0, w - 1), 3].astype(np.int64) << 6
    A = np.where(my[:, None] & mx[None, :], sub, 0)
    if kern[0] == "fir":
        A = fir_1d(fir_1d(A, kern[1], 1), kern[1], 0)
    elif kern[0] == "box":
        for hh in kern[1]:
            A = box_1d(A, hh, 1)
        for hh in kern[1]:
            A = box_1d(A, hh, 0)
    S = np.clip(A, 0, 16320)
    k = op * (color >> 24)
    sh = np.zeros(img.shape, np.uint8)
    sh[..., 0], sh[..., 1], sh[..., 2] = color & 255, (color >> 8) & 255, (color >> 16) & 255
    sh[..., 3] = (S * k + 816000) // 1632000
    return sh if only else blend(sh, img, 0)


def suite_shadow(R, a):
    rng = np.random.default_rng(27)
    img = natural(70, 52, 27)
    img[:, :20, 3] = 0
    img[:8, :, 3] = 0
    for col, ang, d, r, op, only in ((0xFF000000, 45.0, 10, 10, 50, 0), (0xFF3060C0, -120.0, 25, 3, 100, 0),
                                     (0x80000000, 0.0, 0, 0, 70, 1), (0xFFFFFFFF, 170.0, 100, 50, 30, 0),
                                     (0xFF000000, 90.0, 6, 1, 100, 1)):
        got, _ = R.fx("drop_shadow", img, {"color": "%08X" % col, "angle": ang, "distance": d,
                                           "blur_radius": r, "opacity": op, "shadow_only": only}, tile=64)
        compare(f"shadow c={col:08X} a={ang} d={d} r={r} o={op} only={only}", got,
                ref_shadow(img, col, ang, d, r, op, only))
    small = rand_img(rng, 2, 3)
    got, _ = R.fx("drop_shadow", small, {"distance": 1, "blur_radius": 100})
    compare("shadow small", got, ref_shadow(small, 0xFF000000, 45.0, 1, 100, 50, 0))

# ================================================================== suites

def suite_gaussian(R, a):
    rng = np.random.default_rng(5)
    img = natural(97, 61, 3)
    for r in (0, 1, 2, 3, 5, 8, 9, 12, 13, 20, 50):
        got, _ = R.fx("gaussian_blur", img, {"radius": r}, tile=64)
        compare(f"gaussian r={r}", got, img if r == 0 else ref_sep(img, kern_gauss(var12(r))))
    small = rand_img(rng, 7, 5)
    for r in (1, 13, 200):
        got, _ = R.fx("gaussian_blur", small, {"radius": r})
        compare(f"gaussian small r={r}", got, ref_sep(small, kern_gauss(var12(r))))
    one = rand_img(rng, 1, 1, "opaque")
    got, _ = R.fx("gaussian_blur", one, {"radius": 200})
    compare("gaussian 1x1", got, one)


def suite_square(R, a):
    rng = np.random.default_rng(6)
    img = natural(83, 57, 4)
    for r in (0, 1, 4, 30):
        got, _ = R.fx("square_blur", img, {"radius": r}, tile=64)
        ref = img if r == 0 else ref_sep(img, ("box", [r]))
        compare(f"square r={r}", got, ref)
    small = rand_img(rng, 5, 9)
    got, _ = R.fx("square_blur", small, {"radius": 200})
    compare("square small r=200", got, ref_sep(small, ("box", [200])))


def suite_unfocus(R, a):
    rng = np.random.default_rng(7)
    img = natural(71, 53, 5)
    for r in (1, 2, 4, 11):
        got, _ = R.fx("unfocus", img, {"radius": r}, tile=64)
        compare(f"unfocus r={r}", got, ref_unfocus(img, r))
    small = rand_img(rng, 6, 4)
    got, _ = R.fx("unfocus", small, {"radius": 200})
    compare("unfocus small r=200", got, ref_unfocus(small, 200))


def suite_median(R, a):
    rng = np.random.default_rng(8)
    img = natural(61, 43, 6)
    for r, pct in ((1, 50), (3, 50), (5, 0), (5, 100), (6, 25), (10, 50)):
        got, _ = R.fx("median", img, {"radius": r, "percentile": pct}, tile=64)
        compare(f"median r={r} p={pct}", got, ref_median(img, r, pct))
    small = rand_img(rng, 9, 6)
    for r in (1, 200):
        got, _ = R.fx("median", small, {"radius": r, "percentile": 50})
        compare(f"median small r={r}", got, ref_median(small, min(r, 20), 50))


def suite_motion(R, a):
    rng = np.random.default_rng(9)
    img = natural(90, 64, 7)
    for ang, d, cen in ((25.0, 10, 1), (0.0, 7, 0), (90.0, 12, 1), (-37.5, 30, 0), (180.0, 1, 1),
                        (45.0, 2, 1), (133.0, 60, 1)):
        got, _ = R.fx("motion_blur", img, {"angle": ang, "distance": d, "centered": cen}, tile=64)
        compare(f"motion a={ang} d={d} c={cen}", got, ref_motion(img, ang, d, cen))
    small = rand_img(rng, 5, 4)
    for d in (3, 200):
        got, _ = R.fx("motion_blur", small, {"angle": 30.0, "distance": d, "centered": 1})
        compare(f"motion small d={d}", got, ref_motion(small, 30.0, d, 1))
    one = rand_img(rng, 1, 1, "opaque")
    got, _ = R.fx("motion_blur", one, {"angle": 30.0, "distance": 5})
    compare("motion 1x1", got, one)


def suite_zoom(R, a):
    rng = np.random.default_rng(10)
    img = natural(80, 57, 8)
    for amt, ox, oy in ((10, 0.0, 0.0), (0, 0.0, 0.0), (55, 0.3, -0.7), (100, -1.0, 1.0), (7, 1.5, 0.2)):
        got, _ = R.fx("zoom_blur", img, {"amount": amt, "offset": f"{ox},{oy}"}, tile=64)
        compare(f"zoom a={amt} o={ox},{oy}", got, ref_zoom(img, amt, ox, oy))
    small = rand_img(rng, 3, 2)
    got, _ = R.fx("zoom_blur", small, {"amount": 100})
    compare("zoom small", got, ref_zoom(small, 100, 0.0, 0.0))


def suite_radial(R, a):
    rng = np.random.default_rng(11)
    img = natural(50, 37, 9)
    for ang, ox, oy, q in ((2.0, 0.0, 0.0, 2), (30.0, 0.4, -0.2, 1), (360.0, 0.0, 0.0, 1),
                           (11.5, -1.0, 1.0, 3)):
        got, _ = R.fx("radial_blur", img, {"angle": ang, "offset": f"{ox},{oy}", "quality": q}, tile=32)
        compare(f"radial a={ang} o={ox},{oy} q={q}", got, ref_radial(img, ang, ox, oy, q))
    small = rand_img(rng, 2, 3)
    got, _ = R.fx("radial_blur", small, {"angle": 90.0, "quality": 1})
    compare("radial small", got, ref_radial(small, 90.0, 0.0, 0.0, 1))


def suite_fragment(R, a):
    rng = np.random.default_rng(12)
    img = natural(70, 49, 10)
    for n, d, rot in ((4, 8, 0.0), (2, 0, 0.0), (7, 13, 33.3), (50, 100, 270.0), (3, 30, -45.0)):
        got, _ = R.fx("fragment_blur", img, {"fragments": n, "distance": d, "rotation": rot}, tile=64)
        compare(f"fragment n={n} d={d} r={rot}", got, ref_fragment(img, n, d, rot))
    small = rand_img(rng, 1, 1)
    got, _ = R.fx("fragment_blur", small, {"distance": 5})
    compare("fragment 1x1", got, ref_fragment(small, 4, 5, 0.0))


def suite_surface(R, a):
    rng = np.random.default_rng(13)
    img = natural(57, 41, 12)
    for r, t in ((6, 15), (1, 1), (3, 60), (9, 100), (4, 5)):
        got, _ = R.fx("surface_blur", img, {"radius": r, "threshold": t}, tile=64)
        compare(f"surface r={r} t={t}", got, ref_surface(img, r, t))
    small = rand_img(rng, 5, 7)
    got, _ = R.fx("surface_blur", small, {"radius": 100, "threshold": 50})
    compare("surface small r=100", got, ref_surface(small, 12, 50))


def suite_bokeh(R, a):
    rng = np.random.default_rng(14)
    img = natural(66, 45, 13)
    for r, i, g in ((25, 50, 50), (3, 0, 0), (7, 100, 100), (12, 70, 20), (1, 50, 50)):
        got, _ = R.fx("bokeh_blur", img, {"radius": r, "intensity": i, "granularity": g}, tile=64)
        compare(f"bokeh r={r} i={i} g={g}", got, ref_bokeh(img, r, i, g), tol=1)
    small = rand_img(rng, 4, 3)
    got, _ = R.fx("bokeh_blur", small, {"radius": 100, "intensity": 100})
    compare("bokeh small r=100", got, ref_bokeh(small, 100, 100, 50), tol=1)


def suite_sketch(R, a):
    rng = np.random.default_rng(15)
    img = natural(64, 48, 14)
    for r, ang, sp, seed in ((10, 45.0, 60, 0), (1, 0.0, 0, 5), (5, -30.0, 180, 123456),
                             (17, 91.0, 30, 7)):
        got, _ = R.fx("sketch_blur", img, {"radius": r, "angle": ang, "spread": sp, "seed": seed},
                      tile=64)
        compare(f"sketch r={r} a={ang} s={sp} seed={seed}", got, ref_sketch(img, r, ang, sp, seed))
    small = rand_img(rng, 3, 2)
    got, _ = R.fx("sketch_blur", small, {"radius": 50, "seed": 3})
    compare("sketch small r=50", got, ref_sketch(small, 50, 45.0, 60, 3))

def suite_glow(R, a):
    rng = np.random.default_rng(16)
    img = natural(71, 50, 15)
    for r, b, c in ((6, 10, 10), (1, 0, 0), (20, -100, 100), (13, 100, -100), (3, 50, 99)):
        got, _ = R.fx("glow", img, {"radius": r, "brightness": b, "contrast": c}, tile=64)
        compare(f"glow r={r} b={b} c={c}", got, ref_glow(img, r, b, c))
    small = rand_img(rng, 3, 5)
    got, _ = R.fx("glow", small, {"radius": 20})
    compare("glow small", got, ref_glow(small, 20, 10, 10))


def suite_soften(R, a):
    rng = np.random.default_rng(17)
    img = natural(60, 44, 16)
    for s, l, w in ((5, 0, 10), (0, 0, 0), (10, -20, 20), (3, 17, 5), (7, -3, 0)):
        got, _ = R.fx("soften_portrait", img, {"softness": s, "lighting": l, "warmth": w}, tile=64)
        compare(f"soften s={s} l={l} w={w}", got, ref_soften(img, s, l, w))
    small = rand_img(rng, 2, 2)
    got, _ = R.fx("soften_portrait", small, {"softness": 10})
    compare("soften small", got, ref_soften(small, 10, 0, 10))


def suite_sharpen(R, a):
    rng = np.random.default_rng(18)
    img = natural(55, 40, 17)
    for amt in (1, 4, 10, 20):
        got, _ = R.fx("sharpen", img, {"amount": amt}, tile=64)
        compare(f"sharpen a={amt}", got, ref_sharpen(img, amt))
    small = rand_img(rng, 4, 3)
    got, _ = R.fx("sharpen", small, {"amount": 20})
    compare("sharpen small", got, ref_sharpen(small, 20))


def suite_redeye(R, a):
    rng = np.random.default_rng(19)
    img = natural(80, 60, 18)
    rnd = rand_img(rng, 64, 64)
    for t, s in ((70, 90), (0, 0), (30, 100), (100, 50)):
        got, _ = R.fx("red_eye_removal", img, {"tolerance": t, "saturation": s}, tile=64)
        compare(f"redeye t={t} s={s}", got, ref_redeye(img, t, s))
        got, _ = R.fx("red_eye_removal", rnd, {"tolerance": t, "saturation": s}, tile=64)
        compare(f"redeye random t={t} s={s}", got, ref_redeye(rnd, t, s))


def suite_straighten(R, a):
    rng = np.random.default_rng(20)
    img = natural(73, 51, 19)
    for ang, z in ((0.0, 1), (10.0, 1), (-7.5, 1), (45.0, 1), (-30.0, 0), (3.3, 0)):
        got, _ = R.fx("straighten", img, {"angle": ang, "zoom": z}, tile=64)
        compare(f"straighten a={ang} z={z}", got, ref_straighten(img, ang, z))
    small = rand_img(rng, 1, 1)
    got, _ = R.fx("straighten", small, {"angle": 20.0})
    compare("straighten 1x1", got, ref_straighten(small, 20.0, 1))


def suite_vignette(R, a):
    rng = np.random.default_rng(21)
    img = natural(90, 61, 20)
    for ox, oy, rad, amt, col in ((0.0, 0.0, 0.5, 1.0, 0xFF000000), (0.3, -0.5, 0.25, 0.7, 0xFFFFFFFF),
                                  (-1.0, 1.0, 4.0, 1.0, 0x80FF0000), (0.0, 0.0, 0.1, 0.0, 0xFF000000)):
        got, _ = R.fx("vignette", img, {"offset": f"{ox},{oy}", "radius": rad, "density": amt,
                                        "color": "%08X" % col}, tile=64)
        compare(f"vignette o={ox},{oy} r={rad} a={amt} c={col:08X}", got,
                ref_vignette(img, ox, oy, rad, amt, col))
    small = rand_img(rng, 1, 2)
    got, _ = R.fx("vignette", small, {})
    compare("vignette small", got, ref_vignette(small, 0.0, 0.0, 0.5, 1.0, 0xFF000000))

# effect name -> (suite, default params for the generic checks)
EFFECTS = {
    "gaussian_blur": (suite_gaussian, {"radius": 7}),
    "square_blur": (suite_square, {"radius": 5}),
    "unfocus": (suite_unfocus, {"radius": 6}),
    "median": (suite_median, {"radius": 4, "percentile": 50}),
    "motion_blur": (suite_motion, {"angle": 25.0, "distance": 20, "centered": 1}),
    "zoom_blur": (suite_zoom, {"amount": 30, "offset": "0.2,-0.1"}),
    "radial_blur": (suite_radial, {"angle": 10.0, "offset": "0.1,0.1", "quality": 2}),
    "fragment_blur": (suite_fragment, {"fragments": 5, "distance": 12, "rotation": 20.0}),
    "surface_blur": (suite_surface, {"radius": 6, "threshold": 15}),
    "bokeh_blur": (suite_bokeh, {"radius": 12, "intensity": 60, "granularity": 50}),
    "sketch_blur": (suite_sketch, {"radius": 8, "angle": 45.0, "spread": 60, "seed": 1}),
    "glow": (suite_glow, {"radius": 6, "brightness": 10, "contrast": 10}),
    "soften_portrait": (suite_soften, {"softness": 5, "lighting": 0, "warmth": 10}),
    "sharpen": (suite_sharpen, {"amount": 10}),
    "red_eye_removal": (suite_redeye, {"tolerance": 70, "saturation": 90}),
    "straighten": (suite_straighten, {"angle": 12.0, "zoom": 1}),
    "vignette": (suite_vignette, {"offset": "0.1,-0.2", "radius": 0.5, "density": 1.0}),
    "oil_painting": (suite_oil, {"brush_size": 3, "coarseness": 50}),
    "pencil_sketch": (suite_pencil, {"pencil_tip_size": 2, "color_range": 0}),
    "ink_sketch": (suite_ink, {"ink_outline": 50, "coloring": 50}),
    "add_noise": (suite_addnoise, {"intensity": 64, "color_saturation": 100, "coverage": 100, "seed": 3}),
    "reduce_noise": (suite_reducenoise, {"radius": 6, "strength": 0.4}),
    "drop_shadow": (suite_shadow, {"angle": 45.0, "distance": 10, "blur_radius": 10, "opacity": 50}),
}


def generic_checks(R, name, params):
    """tile / thread determinism, ROI, cancel, transparent colour bleeding"""
    img = natural(203, 131, 11)
    base, _ = R.fx(name, img, params, tile=4096, threads=1)
    if not check(base is not None, f"{name}: whole run"):
        return
    for tile, thr in ((64, 1), (256, 1), (37, 0), (64, 0)):
        got, _ = R.fx(name, img, params, tile=tile, threads=thr)
        compare(f"{name} tile={tile} threads={thr or 'N'}", got, base)
    got, _ = R.fx(name, img, params, tile=64, roi=(13, 17, 150, 100))
    m = np.zeros(img.shape[:2], np.int32)
    m[17:100, 13:150] = 1
    compare(f"{name} roi", got, base, mask=m)
    rc, out = R.run(["--fx", name, "--in", R.path("in_0.raw"), "--cancel"])
    check("code=-14" in out, f"{name}: cancel -> {out.strip()[:80]}")
    # colour of fully transparent pixels must not matter
    img2 = img.copy()
    t = img2[..., 3] == 0
    img2[t, :3] = 255 - img2[t, :3]
    got2, _ = R.fx(name, img2, params, tile=4096)
    if got2 is not None:
        vis = (base[..., 3] > 0) | (got2[..., 3] > 0)
        compare(f"{name} transparent colour", got2, base, mask=vis.astype(np.int32))


def extremes(H, names):
    """every property at its minimum and maximum (others default) on 1x1, 2x3 and 17x9
    images plus all-min / all-max: every run must succeed (guards, source unchanged) and be
    identical for tile 4 / 1 thread and tile 64 / the pool"""
    rng = np.random.default_rng(77)
    imgs = [rand_img(rng, 1, 1), rand_img(rng, 2, 3), rand_img(rng, 17, 9)]
    info = H.effects()
    for n in names:
        props = info.get(n, {}).get("props", {})
        sets = [{}]
        lo, hi = {}, {}
        for k, pr in props.items():
            t = pr.get("type")
            if t in ("int", "dec", "angle", "seed"):
                v0, v1 = pr["min"], pr["max"]
                sets += [{k: v0}, {k: v1}]
                lo[k], hi[k] = v0, v1
            elif t == "bool":
                sets += [{k: 0}, {k: 1}]
                lo[k], hi[k] = 0, 1
            elif t == "pan":
                sets += [{k: "-1,-1"}, {k: "1,1"}]
                lo[k], hi[k] = "-1,-1", "1,1"
            elif t == "color":
                sets += [{k: "00000000"}, {k: "FFFFFFFF"}]
        sets += [lo, hi]
        # hostile values far outside the ranges (clamped / sanitised by the kernels)
        for k, pr in props.items():
            t = pr.get("type")
            if t in ("int", "seed"):
                sets += [{k: -2147483647}, {k: 2147483647}]
            elif t in ("dec", "angle"):
                sets += [{k: -1e300}, {k: 1e300}]
            elif t == "pan":
                sets += [{k: "-1e300,1e300"}]
        specs = []
        for img in imgs:
            for p in sets:
                specs.append(fc.Spec(n, img, p, tile=4, threads=1))
                specs.append(fc.Spec(n, img, p, tile=64))
        res = H.runs(specs)
        bad = 0
        for i in range(0, len(res), 2):
            a, b = res[i], res[i + 1]
            ok = a.ok and b.ok and a.out is not None and b.out is not None and \
                np.array_equal(a.out, b.out)
            if not ok:
                bad += 1
                if bad <= 3:
                    print(f"  extremes {n} {specs[i].params} {specs[i].img.shape}: {a.line} | {b.line}")
        check(bad == 0, f"{n}: {bad} extreme-parameter runs failed")


def shots(R, outdir, names):
    outdir = Path(outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    img = natural(480, 320, 1)
    write_png(outdir / "source.png", img)
    for n in names:
        got, line = R.fx(n, img, EFFECTS[n][1] if n in EFFECTS else {}, tile=128, threads=0)
        if got is None:
            print("  shot failed", n, line)
            continue
        write_png(outdir / f"{n}.png", got)
    print("shots in", outdir)


def bench(R, names):
    img = natural(4000, 3000, 2, alpha=False)
    for n in names:
        got, line = R.fx(n, img, EFFECTS[n][1], tile=256, threads=0, extra=["--bench", "3"])
        print(f"  {n}: {line}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="win", choices=("win", "linux", "both"))
    ap.add_argument("--out", default="build/fxb")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--only", default="")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--shots", default="")
    ap.add_argument("--no-generic", action="store_true")
    a = ap.parse_args()
    targets = ("win", "linux") if a.target == "both" else (a.target,)
    names = [n for n in EFFECTS if not a.only or n in a.only.split(",")]
    for t in targets:
        if not a.no_build:
            build.build_app("efftest", t, ROOT / a.out)
        R = Runner(t, ROOT / a.out)
        H = fc.Harness(t, ROOT / a.out, work=ROOT / a.out / "fxb_fc" / t)
        rep = fc.Report(t)
        if a.shots:
            shots(R, a.shots, names)
            continue
        if a.bench:
            bench(R, names)
            continue
        info = H.effects()
        if not a.no_generic:
            print(f"[{t}] extreme parameters")
            extremes(H, names)
        for n in names:
            t0 = time.time()
            print(f"[{t}] {n}")
            check(n in info, f"{n}: not registered")
            EFFECTS[n][0](R, a)
            if not a.no_generic:
                generic_checks(R, n, EFFECTS[n][1])
                # FXA's generic checks: determinism, fill coverage, ROI, selection, cancel
                img = fc.with_alpha_hole(natural(150, 110, 31))
                slow = bool(info.get(n, {}).get("flags", 0) & fc.FXF_SLOW)
                fails = fc.generic_checks(H, n, img, EFFECTS[n][1], rep, slow=slow)
                NCHECK[0] += 1
                FAILS.extend(fails)
            print(f"  ({time.time() - t0:.1f} s)")
    print(f"{NCHECK[0]} checks, {len(FAILS)} failures")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
