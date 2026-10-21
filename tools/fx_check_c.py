#!/usr/bin/env python3
"""fx_check_c.py -- checks of the FXC effect kernels (ASMLIB/src/fx: fx_distort.s, fx_render.s,
fx_warp.s, fx_perlin.s) through apps/efftest, on top of FXA's harness (tools/fx_check.py).

  python tools/fx_check_c.py [--target win|linux|both] [--out build/fxc] [--no-build]
                             [--only NAME,...] [--quick] [--bench] [--view DIR]

Every effect has a numpy reference with the documented semantics:
bit-exact for the integer pipelines (Pixelate, Morphology) and the f32 warp sampler / noise /
fractal arithmetic replicated operation by operation (tolerance 1..2 where the reference uses
numpy's sin / cos / log instead of the kernels' polynomials).  Besides the references
(fx_check.generic_checks): tile / thread determinism (tiles 64 / 256 / 37 / whole, threads 1 /
pool identical), every ROI pixel written, nothing outside the ROI, selection lerp, JOB_CANCEL;
plus extreme parameters, 1 x 1 images, transparent pixels, SSE2 vs AVX2 (fractals).
--view DIR writes PNGs of every effect on natural-looking test images; --bench times 4000 x 3000
runs against the performance targets.
"""

import argparse
import math
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402
import fx_check as fc  # noqa: E402
import blend_check as bc  # noqa: E402

F32 = np.float32
EDGES = ["clamp", "wrap", "reflect", "primary", "secondary", "transparent", "original"]
BLENDS = ["normal", "multiply", "additive", "color_burn", "color_dodge", "reflect", "glow",
          "overlay", "difference", "negation", "lighten", "darken", "screen", "xor"]
SCHEMES = ["classic", "grayscale", "palette", "rainbow"]


# ================================================================== small helpers

def u32(v):
    return np.asarray(v, np.int64) & 0xFFFFFFFF


def fmix(h):
    h = u32(h)
    h ^= h >> 16
    h = (h * 0x85EBCA6B) & 0xFFFFFFFF
    h ^= h >> 13
    h = (h * 0xC2B2AE35) & 0xFFFFFFFF
    h ^= h >> 16
    return h


def hash32(x, y, seed, stream):
    """fx_hash32 (fx_common.s)"""
    h = (u32(x) * 0x8DA6B343) ^ (u32(y) * 0xD8163841) ^ (u32(seed) * 0xCB1AB31F) \
        ^ (u32(stream) * 0x165667B1)
    return fmix(h & 0xFFFFFFFF)


def pack(b, g, r, a):
    return np.stack([b, g, r, a], -1).astype(np.uint8)


def to_u32(img):
    return np.ascontiguousarray(img).view(np.uint32)[..., 0]


def from_u32(p):
    return np.ascontiguousarray(p.astype(np.uint32)).view(np.uint8).reshape(p.shape + (4,))


def colvec(argb):
    return np.array([argb & 255, (argb >> 8) & 255, (argb >> 16) & 255, (argb >> 24) & 255], F32)


def floor_i(v):
    return np.floor(v)


def col(p, key, default):
    """colour property: hex string AARRGGBB (efftest syntax) or int"""
    v = p.get(key, default)
    return int(v, 16) if isinstance(v, str) else int(v)


# ================================================================== warp framework reference

def rgss(q):
    n = q * q
    if n == 1:
        return [(0.0, 0.0)]
    out = []
    for i in range(n):
        y = (i + 1) / (n + 1.0)
        x = y * q
        x -= float(int(x))
        out.append((x - 0.5, y - 0.5))
    return out


def premul_vec(px):
    """(..., 4) uint8 straight -> f32 (b a, g a, r a, a)"""
    p = px.astype(F32)
    a = p[..., 3:4]
    t = p.copy()
    t[..., 3] = 1
    return t * a


def edge_idx(v, n, mode):
    if mode == 1:
        return np.mod(v, n)
    if mode == 2:
        m = np.mod(v, 2 * n)
        return np.where(m >= n, 2 * n - 1 - m, m)
    return np.clip(v, 0, n - 1)


def bilinear(img, sx, sy, mode, cv1, cv2, selfvec):
    H, W = img.shape[:2]
    sx = np.clip(np.nan_to_num(sx, nan=-2.0 ** 30), -2.0 ** 30, 2.0 ** 30)
    sy = np.clip(np.nan_to_num(sy, nan=-2.0 ** 30), -2.0 ** 30, 2.0 ** 30)
    out_mask = None
    m = mode
    if mode >= 3:
        out_mask = (sx < -0.5) | (sy < -0.5) | (sx >= W - 0.5) | (sy >= H - 0.5)
        m = 0
    ix = floor_i(sx)
    iy = floor_i(sy)
    fx = (sx - ix).astype(F32)
    fy = (sy - iy).astype(F32)
    ix = ix.astype(np.int64)
    iy = iy.astype(np.int64)
    one = F32(1)
    wx0, wy0 = one - fx, one - fy
    ws = [wx0 * wy0, fx * wy0, wx0 * fy, fx * fy]
    x0, x1 = edge_idx(ix, W, m), edge_idx(ix + 1, W, m)
    y0, y1 = edge_idx(iy, H, m), edge_idx(iy + 1, H, m)
    acc = np.zeros(sx.shape + (4,), F32)
    for (yy, xx), w in zip(((y0, x0), (y0, x1), (y1, x0), (y1, x1)), ws):
        acc = acc + premul_vec(img[yy, xx]) * w[..., None]
    if out_mask is not None:
        if mode == 3:
            col = np.broadcast_to(cv1, acc.shape)
        elif mode == 4:
            col = np.broadcast_to(cv2, acc.shape)
        elif mode == 6:
            col = selfvec
        else:
            col = np.zeros_like(acc)
        acc = np.where(out_mask[..., None], col, acc)
    return acc


def finish(acc, n):
    sa = acc[..., 3]
    den = np.stack([sa, sa, sa, np.full(sa.shape, F32(n))], -1)
    with np.errstate(divide="ignore", invalid="ignore"):
        q = acc / den
    q = np.rint(np.nan_to_num(q, nan=0.0, posinf=0.0, neginf=0.0))
    out = np.clip(q, 0, 255).astype(np.uint8)
    zero = (sa <= 0) | (out[..., 3] == 0)
    out[zero] = 0
    return out


def warp_ref(img, mapfn, q=1, edge=0, col1=0xFF000000, col2=0xFFFFFFFF, center=None, rin=0.0):
    """mapfn(px, py, s, X, Y) -> (sx_rel, sy_rel, ident) over f64 arrays"""
    H, W = img.shape[:2]
    cx, cy = center if center is not None else ((W - 1) * 0.5, (H - 1) * 0.5)
    Y, X = np.mgrid[0:H, 0:W]
    dx = X.astype(np.float64) - cx
    dy = Y.astype(np.float64) - cy
    selfvec = premul_vec(img)
    acc = np.zeros((H, W, 4), F32)
    offs = rgss(q)
    cv1 = premul_vec(np.frombuffer(np.uint32(col1).tobytes(), np.uint8)[None])[0]
    cv2 = premul_vec(np.frombuffer(np.uint32(col2).tobytes(), np.uint8)[None])[0]
    for s, (ox, oy) in enumerate(offs):
        px = dx + ox
        py = dy + oy
        sx, sy, ident = mapfn(px, py, s, X, Y)
        smp = bilinear(img, sx + cx, sy + cy, edge, cv1, cv2, selfvec)
        smp = np.where(ident[..., None], selfvec, smp)
        acc = acc + smp
    out = finish(acc, len(offs))
    if rin > 0:
        lim = (rin + 0.75) ** 2
        far = dx * dx + dy * dy > lim
        out[far] = img[far]
    return out


def pan_center(W, H, pan=(0.0, 0.0), bounds=None):
    bx0, by0, bx1, by1 = bounds or (0, 0, W, H)
    px = min(max(pan[0], -2.0), 2.0)
    py = min(max(pan[1], -2.0), 2.0)
    cx = (px + 1.0) * (bx1 - bx0) * 0.5 + (bx0 - 0.5)
    cy = (py + 1.0) * (by1 - by0) * 0.5 + (by0 - 0.5)
    return cx, cy


def rdef(W, H):
    return min(W, H) * 0.5


# ----------------------------------------------------------------- warp effects

def ref_bulge(img, p):
    H, W = img.shape[:2]
    a = min(max(int(p.get("amount", 45)), -200), 100) * 0.01
    R = rdef(W, H)
    R2 = R * R

    def mp(x, y, s, X, Y):
        r2 = x * x + y * y
        ident = r2 >= R2
        with np.errstate(invalid="ignore"):
            t = np.sqrt(r2) / R
        rs = 1.0 - t
        rs = rs * rs * a
        f = 1.0 - rs
        return x * f, y * f, ident
    return warp_ref(img, mp, int(p.get("quality", 2)), center=pan_center(W, H, p.get("offset", (0, 0))),
                    rin=R)


def ref_twist(img, p):
    H, W = img.shape[:2]
    am = min(max(int(p.get("amount", 45)), -200), 200)
    k = (am * am if am >= 0 else -am * am) * 0.01
    size = min(max(float(p.get("size", 1.0)), 0.01), 2.0)
    R = size * rdef(W, H)
    R2 = R * R

    def mp(x, y, s, X, Y):
        r2 = x * x + y * y
        ident = r2 >= R2
        t = 1.0 - np.sqrt(r2) / R
        phi = t * t * t * k
        sn, cs = np.sin(phi), np.cos(phi)
        return x * cs - y * sn, x * sn + y * cs, ident
    return warp_ref(img, mp, int(p.get("quality", 2)), center=pan_center(W, H, p.get("offset", (0, 0))),
                    rin=R)


def ref_polar(img, p):
    H, W = img.shape[:2]
    amt = min(max(float(p.get("amount", 4.0)), -4.0), 4.0)
    R2 = rdef(W, H) ** 2

    def mp(x, y, s, X, Y):
        r2 = np.maximum(x * x + y * y, 1e-12)
        f = (R2 / r2 - 1.0) * amt + 1.0
        return x * f, y * f, np.zeros(x.shape, bool)
    return warp_ref(img, mp, int(p.get("quality", 2)), EDGES.index(p.get("edge_behavior", "reflect")),
                    col(p, "primary_color", 0xFF000000), col(p, "secondary_color", 0xFFFFFFFF),
                    center=pan_center(W, H, p.get("offset", (0, 0))))


def ref_tile(img, p):
    H, W = img.shape[:2]
    ang = float(p.get("angle", 30.0)) * 0.017453292519943295769
    c, sn = math.cos(ang), math.sin(ang)
    size = min(max(int(p.get("tile_size", 40)), 2), 200)
    sc = 3.14159265358979323846 / size
    k = float(min(max(int(p.get("curvature", 8)), -100), 100))

    def mp(x, y, s_, X, Y):
        s = x * c + y * sn
        t = y * c - x * sn
        with np.errstate(divide="ignore", invalid="ignore"):
            s = np.tan(s * sc) * k + s
            t = np.tan(t * sc) * k + t
        return s * c - t * sn, s * sn + t * c, np.zeros(x.shape, bool)
    return warp_ref(img, mp, int(p.get("quality", 2)), EDGES.index(p.get("edge_behavior", "wrap")),
                    col(p, "primary_color", 0xFF000000), col(p, "secondary_color", 0xFFFFFFFF))


# ----------------------------------------------------------------- noise reference

GTAB = np.array([[math.sqrt(2) * math.cos(2 * math.pi * (k + 0.5) / 16),
                  math.sqrt(2) * math.sin(2 * math.pi * (k + 0.5) / 16)] for k in range(16)])
GTAB_ASM = np.array([
    [1.3870398453221475, 0.275899379282943], [1.1758756024193588, 0.78569495838710213],
    [0.78569495838710235, 1.1758756024193588], [0.27589937928294311, 1.3870398453221475],
    [-0.27589937928294295, 1.3870398453221475], [-0.7856949583871019, 1.1758756024193588],
    [-1.1758756024193588, 0.78569495838710213], [-1.3870398453221475, 0.2758993792829435],
    [-1.3870398453221475, -0.27589937928294317], [-1.1758756024193591, -0.7856949583871019],
    [-0.78569495838710213, -1.1758756024193588], [-0.27589937928294361, -1.3870398453221473],
    [0.27589937928294306, -1.3870398453221475], [0.78569495838710168, -1.1758756024193591],
    [1.1758756024193588, -0.78569495838710213], [1.3870398453221473, -0.27589937928294367]])
OFS = [(0.0, 0.0), (0.6180339887498949, 0.75487766624669272),
       (0.23606797749978981, 0.50975533249338545), (0.85410196624968471, 0.26463299874007795),
       (0.47213595499957961, 0.019510664986770898), (0.090169943749474513, 0.77438833123346384),
       (0.70820393249936942, 0.5292659974801559), (0.32623792124926432, 0.28414366372684885),
       (0.94427190999915922, 0.039021329973541796), (0.56230589874905412, 0.79389899622023474),
       (0.18033988749894903, 0.54877666246692769), (0.79837387624884393, 0.30365432871361975)]


def fade(f):
    return ((((f * 6.0) - 15.0) * f + 10.0) * f) * f * f


def perlin(x, y, seed, stream):
    """fxn_perlin, vectorised (same operation order)"""
    x = np.clip(np.nan_to_num(x, nan=-2.0 ** 30), -2.0 ** 30, 2.0 ** 30)
    y = np.clip(np.nan_to_num(y, nan=-2.0 ** 30), -2.0 ** 30, 2.0 ** 30)
    ix = np.floor(x)
    iy = np.floor(y)
    fx = x - ix
    fy = y - iy
    ix = ix.astype(np.int64)
    iy = iy.astype(np.int64)
    K = (u32(seed) * 0xCB1AB31F) ^ (u32(stream) * 0x165667B1)
    X0 = (u32(ix) * 0x8DA6B343) & 0xFFFFFFFF
    X1 = (X0 + 0x8DA6B343) & 0xFFFFFFFF
    Y0 = (u32(iy) * 0xD8163841) & 0xFFFFFFFF
    Y1 = (Y0 + 0xD8163841) & 0xFFFFFFFF

    def g(hx, hy, dx, dy):
        h = fmix((hx ^ hy ^ K) & 0xFFFFFFFF) & 15
        G = GTAB_ASM[h]
        return dx * G[..., 0] + dy * G[..., 1]
    g00 = g(X0, Y0, fx, fy)
    g10 = g(X1, Y0, fx - 1.0, fy - 0.0)
    g01 = g(X0, Y1, fx - 0.0, fy - 1.0)
    g11 = g(X1, Y1, fx - 1.0, fy - 1.0)
    u = fade(fx)
    v = fade(fy)
    a = g00 + (g10 - g00) * u
    b = g01 + (g11 - g01) * u
    return a + (b - a) * v


# ----------------------------------------------------------------- dents / frosted / crystalize

def ref_dents(img, p):
    H, W = img.shape[:2]
    R = rdef(W, H)
    scale = min(max(float(p.get("scale", 25.0)), 1.0), 200.0)
    f = 400.0 / (scale * R)
    d = min(max(float(p.get("refraction", 50.0)), 0.0), 200.0) * 0.01 / f
    theta = min(max(float(p.get("tension", 10.0)), 0.0), 100.0) * 0.62831853071795864769
    pers = min(max(float(p.get("roughness", 10.0)), 0.0), 100.0) * 0.01
    seed = int(p.get("seed", 0)) & 0xFFFFFFFF

    def mp(x, y, s, X, Y):
        nx = x * f
        ny = y * f
        sm = np.zeros(x.shape)
        amp = 1.0
        for i in range(4):
            sm = perlin(nx, ny, seed, i) * amp + sm
            if i == 3:
                break
            amp = amp * pers
            if amp < 0.001:
                break
            nx = nx + nx
            ny = ny + ny
        a = sm * theta
        return x - np.sin(a) * d, np.cos(a) * d + y, np.zeros(x.shape, bool)
    return warp_ref(img, mp, int(p.get("quality", 2)))


def ref_frost(img, p):
    H, W = img.shape[:2]
    r = min(max(float(p.get("scatter_radius", 3.0)), 0.0), 200.0)
    invL = 1.0 / (r * 0.5 + 2.0)
    sg = min(max(float(p.get("smoothness", 25.0)), 0.0), 100.0) * 0.01
    seed = int(p.get("seed", 0)) & 0xFFFFFFFF
    cx, cy = (W - 1) * 0.5, (H - 1) * 0.5

    def mp(x, y, s, X, Y):
        wx = np.zeros(x.shape)
        wy = np.zeros(x.shape)
        if sg < 1.0:
            h1 = hash32(X, Y, seed, 2 * s) >> 8
            rad = np.sqrt(h1.astype(np.float64) * 5.9604644775390625e-08)
            h2 = hash32(X, Y, seed, 2 * s + 1) >> 8
            ang = h2.astype(np.float64) * 3.7450702829238412e-07
            w1 = 1.0 - sg
            wx = np.cos(ang) * rad * w1
            wy = np.sin(ang) * rad * w1
        if sg > 0.0:
            ax = (x + cx) * invL
            ay = (y + cy) * invL
            nx = np.clip(perlin(ax, ay, seed, 7) * 1.4, -1.0, 1.0)
            ny = np.clip(perlin(ax, ay, seed, 8) * 1.4, -1.0, 1.0)
            wy = ny * sg + wy
            wx = nx * sg + wx
        return wx * r + x, wy * r + y, np.zeros(x.shape, bool)
    return warp_ref(img, mp, int(p.get("quality", 2)))


def ref_cryst(img, p):
    H, W = img.shape[:2]
    s = float(min(max(int(p.get("cell_size", 8)), 2), 250))
    inv = 1.0 / s
    seed = int(p.get("seed", 0)) & 0xFFFFFFFF
    cx, cy = (W - 1) * 0.5, (H - 1) * 0.5

    def site(ci, cj):
        u = (hash32(ci, cj, seed, 0) >> 8).astype(np.float64) * 5.9604644775390625e-08 + ci
        v = (hash32(ci, cj, seed, 1) >> 8).astype(np.float64) * 5.9604644775390625e-08 + cj
        return u, v

    def mp(x, y, s_, X, Y):
        gx = np.clip(((x + cx) + 0.5) * inv, -1e9, 1e9)
        gy = np.clip(((y + cy) + 0.5) * inv, -1e9, 1e9)
        ci = np.floor(gx).astype(np.int64)
        cj = np.floor(gy).astype(np.int64)

        def search(rr, best, bu, bv):
            for dj in range(-rr, rr + 1):
                for di in range(-rr, rr + 1):
                    u, v = site(ci + di, cj + dj)
                    dd = (u - gx) ** 2 + (v - gy) ** 2
                    better = dd < best
                    best = np.where(better, dd, best)
                    bu = np.where(better, u, bu)
                    bv = np.where(better, v, bv)
            return best, bu, bv
        best = np.full(gx.shape, 1e300)
        bu = np.zeros(gx.shape)
        bv = np.zeros(gx.shape)
        best, bu, bv = search(1, best, bu, bv)
        fx = gx - ci
        fy = gy - cj
        m = np.minimum(np.minimum(fx, 1.0 - fx), np.minimum(fy, 1.0 - fy))
        lim = (m + 1.0) ** 2
        need = ~(lim >= best)
        if need.any():
            b2, u2, v2 = search(2, best, bu, bv)
            best = np.where(need, b2, best)
            bu = np.where(need, u2, bu)
            bv = np.where(need, v2, bv)
        sx = np.clip(np.floor(bu * s), 0, W - 1)
        sy = np.clip(np.floor(bv * s), 0, H - 1)
        return sx - cx, sy - cy, np.zeros(x.shape, bool)
    return warp_ref(img, mp, int(p.get("quality", 2)))


# ----------------------------------------------------------------- pixelate / morphology

def ref_pixelate(img, p):
    H, W = img.shape[:2]
    s = min(max(int(p.get("cell_size", 2)), 1), 100)
    out = np.zeros_like(img)
    a = img[..., 3].astype(np.int64)
    for by in range(0, H, s):
        for bx in range(0, W, s):
            blk = img[by:by + s, bx:bx + s].astype(np.int64)
            ba = blk[..., 3]
            sa = int(ba.sum())
            n = blk.shape[0] * blk.shape[1]
            if sa == 0:
                continue
            aa = (sa + n // 2) // n
            if aa == 0:
                continue
            px = [(int((blk[..., c] * ba).sum()) + sa // 2) // sa for c in range(3)] + [aa]
            out[by:by + s, bx:bx + s] = px
    del a
    return out


def premul_img(img):
    return from_u32(bc.ref_premul(to_u32(img).astype(np.int64)))


def unpremul_img(img):
    return from_u32(bc.ref_unpremul(to_u32(img).astype(np.int64)))


def morph_widths(r, shape):
    w = []
    for d in range(r + 1):
        if shape == 1:
            w.append(r)
        elif shape == 2:
            w.append(r - d)
        else:
            w.append(int(math.sqrt(r * r + r - d * d)))
    return w


def win_max(P, K, op):
    """op over every window of K consecutive columns of P -> P.shape[1] - K + 1 columns"""
    T = P
    k = 1
    while 2 * k <= K:
        T = op(T[:, :-k], T[:, k:])
        k *= 2
    n = P.shape[1] - K + 1
    return op(T[:, :n], T[:, K - k:K - k + n])


def morph_op(pm, r, shape, is_min, valid):
    """max / min over the element; pixels with valid == False do not take part"""
    H, W = pm.shape[:2]
    neutral = 255 if is_min else 0
    src = np.where(valid[..., None], pm, neutral).astype(np.uint8)
    pad = np.full((H + 2 * r, W + 2 * r, 4), neutral, np.uint8)
    pad[r:r + H, r:r + W] = src
    out = np.full((H, W, 4), neutral, np.uint8)
    op = np.minimum if is_min else np.maximum
    ws = morph_widths(r, shape)
    rows = {}
    for dy in range(-r, r + 1):
        w = ws[abs(dy)]
        band = pad[r + dy:r + dy + H, r - w:r + w + W]
        out = op(out, win_max(band, 2 * w + 1, op))
    del rows
    return out


def ref_morph(img, p):
    r = min(max(int(p.get("radius", 5)), 1), 250)
    mode = ["dilate", "erode", "open", "close"].index(p.get("mode", "dilate"))
    shape = ["circle", "square", "diamond"].index(p.get("shape", "circle"))
    ops = {0: [False], 1: [True], 2: [True, False], 3: [False, True]}[mode]
    pm = premul_img(img)
    valid = np.ones(img.shape[:2], bool)
    for is_min in ops:
        pm = morph_op(pm, r, shape, is_min, valid)
    return unpremul_img(pm)


# ----------------------------------------------------------------- clouds / turbulence

def noise_field(W, H, kind, scale, power, detail, seed):
    scale = min(max(int(scale), 2), 1000)
    power = min(max(float(power), 0.0), 1.0)
    inv, amp = [], []
    a, div = 1.0, scale
    lim = 12 if kind == 0 else min(max(int(detail), 1), 12)
    thr = 0.03 if kind == 0 else -1.0
    while len(inv) < lim and a > thr and div > 0:
        inv.append(1.0 / div)
        amp.append(a)
        a = a * power
        div >>= 1
    Y, X = np.mgrid[0:H, 0:W]
    dx = (2 * X - W).astype(np.float64)
    dy = (2 * Y - H).astype(np.float64)
    acc = np.zeros((H, W))
    for i in range(len(inv)):
        n = perlin(dx * inv[i] + OFS[i][0], inv[i] * dy + OFS[i][1], seed, i)
        if kind:
            n = np.abs(n)
        acc = n * amp[i] + acc
    if kind == 0:
        t = (acc + 1.0) * 0.5
    else:
        sa = 0.0
        for v in amp:
            sa += v
        t = acc * (2.0 / sa)
    return np.clip(t, 0.0, 1.0)


def ramp(t, c1, c2):
    v1, v2 = colvec(c1), colvec(c2)
    dc = v2 - v1
    col = t.astype(F32)[..., None] * dc + v1
    return np.clip(np.rint(col), 0, 255).astype(np.uint8)


def ref_noise(img, p, kind):
    H, W = img.shape[:2]
    mode = p.get("blend_mode", "normal")
    mode = BLENDS.index(mode) if isinstance(mode, str) else int(mode)
    if kind == 0:
        t = noise_field(W, H, 0, p.get("scale", 250), p.get("power", 0.5), 0, int(p.get("seed", 0)))
    else:
        t = noise_field(W, H, 1, p.get("scale", 250), p.get("roughness", 0.5), p.get("detail", 8),
                        int(p.get("seed", 0)))
    top = ramp(t, col(p, "primary_color", 0xFF000000), col(p, "secondary_color", 0xFFFFFFFF))
    res = bc.ref_blend(to_u32(img).ravel().astype(np.int64), to_u32(top).ravel().astype(np.int64),
                       mode, 255)
    return from_u32(res.reshape(H, W))


# ----------------------------------------------------------------- fractals

def rainbow(c, inside):
    blk = inside | (c >= 1023.0) | np.isnan(c)
    t = np.maximum(c * 0.015625, -1e9)
    t = t - np.floor(t)
    t = t * 6.0
    sx = np.trunc(t).astype(np.int64)
    f = t - sx
    F = np.rint(f * 255.0).astype(np.int64)
    G = 255 - F
    sx = np.minimum(sx, 5)
    z = np.zeros_like(F)
    full = np.full_like(F, 255)
    r = np.select([sx == 0, sx == 1, sx == 2, sx == 3, sx == 4], [full, G, z, z, F], full)
    g = np.select([sx == 0, sx == 1, sx == 2, sx == 3, sx == 4], [F, full, full, G, z], z)
    b = np.select([sx == 0, sx == 1, sx == 2, sx == 3, sx == 4], [z, z, F, full, full], G)
    out = np.stack([b, g, r, full], -1)
    out[blk] = [0, 0, 0, 255]
    return out


def iterate(zx, zy, cx, cy, kmax, r2):
    n = np.zeros(zx.shape)
    s = np.zeros(zx.shape)
    act = np.ones(zx.shape, bool)
    x, y = zx.copy(), zy.copy()
    k = 0
    old = np.seterr(over="ignore", invalid="ignore")
    while act.any():
        ss = x * x + y * y
        stop = act & ((k >= kmax) | (ss >= r2))
        n[stop] = k
        s[stop] = ss[stop]
        act &= ~stop
        if not act.any():
            break
        x, y = (x * x - y * y) + cx, (y * (x + x)) + cy
        k += 1
    np.seterr(**old)
    return n, s


def ref_fractal(img, p, kind):
    H, W = img.shape[:2]
    if kind == 0:
        fact = min(max(float(p.get("factor", 4.0)), 1.0), 10.0)
        q = min(max(int(p.get("quality", 2)), 1), 5)
        zoom = min(max(float(p.get("zoom", 1.0)), 0.01), 50.0)
        invz = 1.0 / zoom
        count = q * q
        scheme = p.get("color_scheme", "classic")
        invert = False
        K = np.array([768.0, 512.0, 256.0, 0.0])
    else:
        fi = min(max(int(p.get("factor", 1)), 1), 10)
        fact = float(fi)
        q = min(max(int(p.get("quality", 2)), 1), 5)
        z = min(max(int(p.get("zoom", 10)), 0), 50)
        invz = 1.0 / float(1 + 20 * z)
        count = q * q + 1
        scheme = p.get("color_scheme", "classic")
        invert = bool(int(p.get("invert_colors", 0)))
        K = np.array([256.0, 512.0, 768.0, 0.0])
    scheme = SCHEMES.index(scheme) if isinstance(scheme, str) else int(scheme)
    ang = min(max(float(p.get("angle", 0.0)), -36000.0), 36000.0) * 0.017453292519943295769
    sn, cs = math.sin(ang), math.cos(ang)
    invq = 1.0 / q
    invc = 1.0 / count
    invh = 1.0 / H
    c1, c2 = col(p, "primary_color", 0xFF000000), col(p, "secondary_color", 0xFFFFFFFF)
    if scheme == 1:
        c1, c2 = 0xFF000000, 0xFFFFFFFF
    Y, X = np.mgrid[0:H, 0:W]
    bx = (X * 2.0) - W
    by = (Y * 2.0) - H
    sums = np.zeros((H, W, 4), np.int64)
    for i in range(count):
        u = (i * invc + bx) * invh
        t = i * invq
        v = ((t - float(int(t))) + by) * invh
        up = u * cs - v * sn
        vp = u * sn + v * cs
        px = up * invz
        py = vp * invz
        if kind == 0:
            n, s = iterate(px, py, np.full(px.shape, 0.3125), np.full(px.shape, 0.03), 256, 10000.0)
            with np.errstate(divide="ignore"):
                ln = np.log(np.maximum(s, 2.2250738585072014e-308))
            c = (n - (2.0 - 18.420680743952367 / ln)) * fact
            inside = np.zeros(c.shape, bool)
        else:
            cr = px + -0.7453
            ci = py + 0.1127
            xq = cr - 0.25
            yq = ci * ci
            qq = xq * xq + yq
            card = ((xq + qq) * qq) <= yq * 0.25
            bulb = ((cr + 1.0) ** 2 + ci * ci) <= 0.0625
            pre = card | bulb
            kmax = 1023 // fi + 1
            n, s = iterate(np.zeros(cr.shape), np.zeros(cr.shape), np.where(pre, 0.0, cr),
                           np.where(pre, 0.0, ci), kmax, 100000.0)
            inside = pre | (s <= 0)
            with np.errstate(divide="ignore"):
                ln = np.log(np.maximum(s, 2.2250738585072014e-308))
            c = (n - ln * 0.086858896380650365) * fact + 64.0
        if scheme == 0:
            ch = np.clip(c[..., None] - K, 0.0, 255.0)
            ch = np.trunc(np.nan_to_num(ch, nan=0.0)).astype(np.int64)
            ch[inside] = 255
        elif scheme == 3:
            ch = rainbow(c, inside)
        else:
            tt = np.where(inside, 1.0, np.clip(c * 0.0009765625, 0.0, 1.0))
            ch = ramp(tt, c1, c2).astype(np.int64)
        sums += ch
    out = (sums // count).astype(np.uint8)
    if invert:
        out[..., :3] = 255 - out[..., :3]
    return out


# ================================================================== suites

EFFECTS = {
    # name: (reference, [param sets], tolerance, allowed fraction of differing values)
    "bulge": (ref_bulge, [{}, {"amount": -200, "quality": 1}, {"amount": 100, "quality": 3,
                                                             "offset": (0.4, -0.3)}], 1, 0.002),
    "twist": (ref_twist, [{}, {"amount": -120, "size": 0.5, "quality": 1},
                          {"amount": 200, "size": 2.0, "offset": (-0.5, 0.5), "quality": 2}], 2, 0.01),
    "polar_inversion": (ref_polar, [{}] + [{"edge_behavior": e, "amount": a, "quality": 1}
                                           for e, a in (("clamp", 1.0), ("wrap", -2.0),
                                                        ("primary", 3.0), ("secondary", 0.5),
                                                        ("transparent", -4.0), ("original", 1.5))]
                        + [{"primary_color": "80FF0000", "edge_behavior": "primary"}], 2, 0.02),
    "tile_reflection": (ref_tile, [{}, {"angle": -45.0, "tile_size": 17, "curvature": -30,
                                        "edge_behavior": "clamp", "quality": 1},
                                   {"angle": 0.0, "tile_size": 200, "curvature": 100,
                                    "edge_behavior": "transparent", "quality": 3}], 2, 0.03),
    "dents": (ref_dents, [{}, {"scale": 5.0, "refraction": 200.0, "roughness": 100.0,
                               "tension": 100.0, "quality": 1, "seed": 7},
                          {"scale": 200.0, "refraction": 0.0, "quality": 3}], 2, 0.01),
    "frosted_glass": (ref_frost, [{}, {"scatter_radius": 0.0}, {"scatter_radius": 25.0, "smoothness": 0.0,
                                                               "quality": 1, "seed": 99},
                                  {"scatter_radius": 60.0, "smoothness": 100.0, "quality": 3}], 2, 0.01),
    "crystalize": (ref_cryst, [{}, {"cell_size": 2, "quality": 1, "seed": 5},
                               {"cell_size": 57, "quality": 3, "seed": 1234}], 1, 0.002),
    "pixelate": (ref_pixelate, [{}, {"cell_size": 1}, {"cell_size": 7}, {"cell_size": 100}], 0, None),
    "morphology": (ref_morph, [{}] + [{"radius": r, "mode": m, "shape": s}
                                     for r, m, s in ((1, "dilate", "circle"), (3, "erode", "square"),
                                                     (4, "open", "diamond"), (2, "close", "circle"),
                                                     (6, "dilate", "square"), (5, "erode", "diamond"))],
                   0, None),
    "clouds": (lambda img, p: ref_noise(img, p, 0),
               [{}, {"scale": 2, "power": 1.0, "seed": 3}, {"scale": 1000, "power": 0.0},
                {"blend_mode": "overlay", "primary_color": "FF204080", "secondary_color": "80FFE0A0"},
                {"blend_mode": "multiply", "scale": 60, "power": 0.8}], 1, 0.002),
    "turbulence": (lambda img, p: ref_noise(img, p, 1),
                   [{}, {"scale": 30, "roughness": 1.0, "detail": 12, "seed": 11},
                    {"detail": 1, "roughness": 0.0}, {"blend_mode": "screen", "scale": 400}], 1, 0.002),
    "julia_fractal": (lambda img, p: ref_fractal(img, p, 0),
                      [{}, {"factor": 10.0, "zoom": 3.0, "angle": 30.0, "quality": 3},
                       {"quality": 1, "zoom": 0.0}, {"color_scheme": "rainbow"},
                       {"color_scheme": "palette", "primary_color": "FF102040",
                        "secondary_color": "FFFFE080"}, {"color_scheme": "grayscale", "zoom": 50.0}],
                      1, 0.01),
    "mandelbrot_fractal": (lambda img, p: ref_fractal(img, p, 1),
                           [{}, {"zoom": 0, "quality": 1}, {"factor": 3, "angle": -60.0, "invert_colors": 1},
                            {"color_scheme": "rainbow", "zoom": 2}, {"color_scheme": "grayscale", "zoom": 50},
                            {"factor": 10, "quality": 3, "zoom": 1}], 1, 0.01),
}
WARPS = {"bulge", "twist", "polar_inversion", "tile_reflection", "dents"}


def test_images(quick):
    w, h = (96, 72) if quick else (160, 120)
    photo = fc.with_alpha_hole(fc.img_photo(w, h))
    return {"photo": photo, "shapes": fc.img_shapes(w - 13, h - 7)}


def suite(h, rep, name, quick):
    ref, psets, tol, frac = EFFECTS[name]
    imgs = test_images(quick)
    t0 = time.time()
    specs, exps, labels = [], [], []
    for iname, img in imgs.items():
        for ps in psets[: 2 if quick else None]:
            specs.append(fc.Spec(name, img, ps, tile=256, threads=1))
            exps.append(ref(img, ps))
            labels.append(f"{name} {iname} {ps}")
    res = h.runs(specs)
    for r, e, lb in zip(res, exps, labels):
        rep.compare(lb, r, e, tol=tol, frac=frac)
    # generic checks (determinism, fill, ROI, selection, cancel)
    img = imgs["photo"]
    for ps in psets[:1] if quick else psets[:2]:
        fc.generic_checks(h, name, img, ps, rep, roi_independent=True)
    # 1 x 1, 1 x N, N x 1, extreme parameters from the property table
    info = h.effects()[name]["props"]
    ext = [{}]
    for key, pr in info.items():
        if pr["type"] in ("int", "dec", "angle"):
            ext.append({key: pr["min"]})
            ext.append({key: pr["max"]})
    tiny = [fc.img_noise(1, 1, 5), fc.img_noise(1, 9, 6), fc.img_noise(11, 1, 7), fc.img_noise(3, 2, 8)]
    specs = [fc.Spec(name, t, ps, tile=64) for t in tiny for ps in ext[: 3 if quick else None]]
    big = fc.img_noise(70, 50, 9)
    specs += [fc.Spec(name, big, ps, tile=32) for ps in ext]
    for r, s in zip(h.runs(specs), specs):
        rep.check(r.ok, f"{name} extreme {s.img.shape[:2]} {s.params}: {r.line}")
    print(f"[{h.target}] {name}: done ({time.time() - t0:.1f} s)")


def suite_morph_big(h, rep, quick):
    """morphology over several chunks (ROI wider than MO_CW = 512, taller than max(64, 2r))"""
    img = fc.with_alpha_hole(fc.img_photo(600, 150))
    cases = [(1, "dilate", "circle"), (13, "erode", "circle"), (40, "dilate", "square"),
             (29, "open", "diamond"), (21, "close", "square"), (97, "dilate", "circle")]
    if quick:
        cases = cases[:3]
    specs, exps = [], []
    for r, m, s in cases:
        ps = {"radius": r, "mode": m, "shape": s}
        e = ref_morph(img, ps)
        for tile in (100000, 64, 173):
            specs.append(fc.Spec("morphology", img, ps, tile=tile))
            exps.append((f"morphology big {ps} tile {tile}", e))
    for r, (lb, e) in zip(h.runs(specs), exps):
        rep.compare(lb, r, e)
    small = fc.img_noise(9, 7, 3)
    for r in (1, 4, 250):
        ps = {"radius": r, "mode": "close", "shape": "circle"}
        rep.compare(f"morphology tiny {ps}", h.run1("morphology", small, ps), ref_morph(small, ps))


def suite_simd(h, rep):
    """fractals: the AVX2 lanes and the SSE2 lanes give identical images"""
    img = fc.img_noise(120, 90, 4)
    for name, ps in (("mandelbrot_fractal", {"quality": 3}), ("julia_fractal", {"quality": 2, "zoom": 2.0}),
                     ("mandelbrot_fractal", {"zoom": 0, "factor": 7})):
        a = h.run1(name, img, ps)
        # --simd is read once per process (cpu_features caches it): pass it on the process
        # command line, not in the --ops line
        h.proc_args = ["--simd", "sse2"]
        try:
            b = h.run1(name, img, ps)
        finally:
            h.proc_args = []
        rep.check(a.ok and b.ok and np.array_equal(a.out, b.out), f"{name} {ps}: --simd sse2 differs")


def natural(w, h, seed=1):
    """photo-like test image (sky, sun, textured hills, shapes, fine lines, transparent strip)"""
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    v = yy / max(h - 1, 1)
    u = xx / max(w - 1, 1)
    img = np.array([200, 120, 40]) * (1 - v[..., None]) + np.array([120, 190, 250]) * v[..., None]
    d = np.hypot(xx - 0.75 * w, yy - 0.25 * h)
    sun = np.clip((0.09 * min(w, h) - d) / 2.0, 0, 1)[..., None]
    img = img * (1 - sun) + np.array([150, 240, 255]) * sun

    def vnoise(cell):
        gh, gw = h // cell + 3, w // cell + 3
        g = rng.random((gh, gw))
        fy, fx = yy / cell, xx / cell
        iy, ix = fy.astype(int), fx.astype(int)
        ty, tx = fy - iy, fx - ix
        ty, tx = ty * ty * (3 - 2 * ty), tx * tx * (3 - 2 * tx)
        a = g[iy, ix] * (1 - tx) + g[iy, ix + 1] * tx
        b = g[iy + 1, ix] * (1 - tx) + g[iy + 1, ix + 1] * tx
        return a * (1 - ty) + b * ty
    tex = sum(vnoise(c) / (i + 1) for i, c in enumerate((max(w // 8, 4), max(w // 32, 3), 3)))
    tex = (tex - tex.min()) / (np.ptp(tex) + 1e-9)
    hill = 0.55 * h + 0.08 * h * np.sin(u * 7.0) + 0.05 * h * np.sin(u * 17.0 + 1)
    m = np.clip((yy - hill) / 1.5, 0, 1)[..., None]
    grass = np.stack([30 + 40 * tex, 90 + 110 * tex, 40 + 60 * tex], -1)
    img = img * (1 - m) + grass * m
    for (cx, cy, r, col) in ((0.2, 0.7, 0.10, (40, 40, 220)), (0.5, 0.8, 0.07, (220, 60, 60)),
                             (0.35, 0.45, 0.05, (60, 200, 230))):
        dd = np.hypot(xx - cx * w, yy - cy * h)
        a = np.clip((r * min(w, h) - dd) / 1.2, 0, 1)[..., None]
        img = img * (1 - a) + np.array(col) * a
    x0, x1, y0, y1 = int(0.05 * w), int(0.3 * w), int(0.1 * h), int(0.35 * h)
    img[y0:y1, x0:x1] = (235, 235, 235)
    for k in range(y0 + 3, y1 - 2, 4):
        img[k, x0 + 3:x1 - 3] = (20, 20, 20)
    img = np.clip(np.rint(img + rng.normal(0, 3.0, img.shape)), 0, 255)
    a = np.where(u > 0.9, np.clip(255 * (1 - (u - 0.9) / 0.08), 0, 255), 255.0)
    out = np.zeros((h, w, 4), np.uint8)
    out[..., :3] = img.astype(np.uint8)
    out[..., 3] = np.rint(a).astype(np.uint8)
    return out


VIEW = [("bulge", {}), ("twist", {}), ("polar_inversion", {}), ("tile_reflection", {}),
        ("dents", {}), ("frosted_glass", {}), ("crystalize", {}), ("pixelate", {"cell_size": 8}),
        ("morphology", {"radius": 4}), ("morphology", {"radius": 4, "mode": "erode", "shape": "square"}),
        ("clouds", {}), ("clouds", {"blend_mode": "overlay"}), ("turbulence", {}),
        ("julia_fractal", {}), ("mandelbrot_fractal", {}),
        ("mandelbrot_fractal", {"zoom": 0, "color_scheme": "rainbow"})]


def view(h, outdir):
    outdir = Path(outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    img = natural(480, 320, 3)
    fc.write_png(outdir / "source.png", img, bg=True)
    res = h.runs([fc.Spec(n, img, p, tile=128) for n, p in VIEW])
    for (n, p), r in zip(VIEW, res):
        tag = n + "".join(f"_{k}{v}" for k, v in p.items())
        if r.ok:
            fc.write_png(outdir / f"{tag}.png", r.out, bg=True)
        print(f"  {tag}: {r.line[:100]}")


BENCH = [("twist", {}, 1.0), ("bulge", {}, 1.0), ("clouds", {}, 0.4), ("mandelbrot_fractal", {}, 1.5),
         ("morphology", {"radius": 10}, 0.6), ("morphology", {"radius": 10, "shape": "square"}, 0.6),
         ("julia_fractal", {}, None), ("turbulence", {}, None), ("dents", {}, None),
         ("frosted_glass", {}, None), ("crystalize", {}, None), ("polar_inversion", {}, None),
         ("tile_reflection", {}, None), ("pixelate", {"cell_size": 10}, None)]


def bench(h, rep):
    img = natural(4000, 3000, 5)
    specs = [fc.Spec(n, img, p, bench=3, keep_out=False) for n, p, _ in BENCH]
    for (n, p, lim), r in zip(BENCH, h.runs(specs)):
        ms = r.render_us / 1000.0
        verdict = "" if lim is None else ("  ok" if ms <= lim * 1000 else f"  SLOW (target {lim * 1000:.0f} ms)")
        print(f"[{h.target}] bench {n} {p}: {ms:.0f} ms, {r.mpxs} Mpx/s{verdict}")
        rep.check(r.ok, f"bench {n}: {r.line}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="win", choices=["win", "linux", "both"])
    ap.add_argument("--out", default=str(ROOT / "build" / "fxc"))
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--only", default="")
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--view", default="")
    a = ap.parse_args()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    out = Path(a.out)
    if not a.no_build:
        for t in targets:
            build.build_app("efftest", t, out)
    only = [s for s in a.only.split(",") if s]
    total_fail = 0
    for t in targets:
        h = fc.Harness(t, out)
        rep = fc.Report(t)
        t0 = time.time()
        if a.view:
            view(h, Path(a.view) / t)
        names = [n for n in EFFECTS if not only or n in only]
        for n in names:
            suite(h, rep, n, a.quick)
        if not only or "morphology" in only:
            suite_morph_big(h, rep, a.quick)
        if not only or "simd" in only:
            suite_simd(h, rep)
        if a.bench:
            bench(h, rep)
        print(f"[{t}] fx_check_c: {rep.passed} checks passed, {len(rep.fails)} failed "
              f"({time.time() - t0:.0f} s)")
        total_fail += len(rep.fails)
    return 1 if total_fail else 0


if __name__ == "__main__":
    sys.exit(main())
