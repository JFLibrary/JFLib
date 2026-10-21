#!/usr/bin/env python3
"""resample_check.py -- check img/resample.s (rs_resize) and img/mip.s against numpy references.

  python tools/resample_check.py [--target win|linux|both] [--out build/g2] [--quick]
                                 [--bench] [--keep]

The references use the SAME kernel definitions, f64 operation order, 14-bit weight
quantisation / normalisation and integer pipeline as the assembly (see the header of
ASMLIB/src/img/resample.s), so the expected difference is 0; the check allows |d| <= 1
(<= 2 with RS_GAMMA).  The weight tables are also compared bit for bit (op `weights`).

Cases: every filter x source format x destination format x gamma over a set of sizes
(1x1, 1xN, Nx1, up / down / mixed, odd sizes), identity (same size) exactness,
integer-scale exactness (nearest replication, Fant = 2x2 box), transparent pixels with
random colours (no colour may bleed out of fully transparent pixels), huge sizes (65535
wide / tall), thread-split determinism (SPLIT 4 == SPLIT 1), guard bytes around every
surface (rstest reports err -200 when padding changed), mip_down / mip_chain (full,
incremental dirty rectangles, odd sizes, straight + premultiplied sources).
--bench times 4000x3000 -> 2000x1500 for every filter (and mip / gamma).
"""

import argparse
import math
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
WSL_DISTRO = "Ubuntu-24.04"

FMT_PARGB, FMT_A8, FMT_ARGB = 1, 2, 3
RS_NEAREST, RS_BILINEAR, RS_BILINEAR_LQ, RS_BICUBIC, RS_BICUBIC_SMOOTH, RS_LANCZOS3, \
    RS_FANT, RS_ADAPTIVE = range(8)
FILTER_NAMES = ["nearest", "bilinear", "bilinear_lq", "bicubic", "bicubic_smooth",
                "lanczos3", "fant", "adaptive"]
RS_GAMMA = 1
WONE = 16384
PONE = 16320

# ---------------------------------------------------------------- kernels (bit exact)
PI = math.pi
THIRD = 1.0 / 3.0
S3, S5, S7, S9, S11, S13 = (-1.0 / 6.0, 1.0 / 120.0, -1.0 / 5040.0, 1.0 / 362880.0,
                            -1.0 / 39916800.0, 1.0 / 6227020800.0)
M1, M0, N3, N1, N0 = 7.0 / 6.0, 8.0 / 9.0, -7.0 / 18.0, 10.0 / 3.0, 16.0 / 9.0
KID_TRI, KID_CUB5, KID_MIT, KID_LAN, KID_CUB75, KID_FANT = range(6)
CUB = {KID_CUB5: (1.5, 2.5, -0.5, -2.5, -4.0, -2.0),
       KID_CUB75: (1.25, 2.25, -0.75, -3.75, -6.0, -3.0)}
SUP = [1.0, 2.0, 2.0, 3.0, 2.0, 1.0]
FKID = [KID_TRI, KID_TRI, KID_TRI, KID_CUB5, KID_MIT, KID_LAN, KID_FANT, KID_LAN]
FWIDE = [0, 1, 0, 1, 1, 1, 0, 1]


def sinpi(t):
    n = math.floor(t + 0.5)
    r = t - n
    z = r * PI
    z2 = z * z
    p = S13
    for c in (S11, S9, S7, S5, S3, 1.0):
        p = p * z2 + c
    s = z * p
    return -s if (n & 1) else s


def kern(kid, x):
    ax = abs(x)
    if kid == KID_TRI:
        return max(1.0 - ax, 0.0)
    if kid in CUB:
        a2, a3, a, a5, a8, a4 = CUB[kid]
        if ax < 1.0:
            p = a2 * ax
            p = p - a3
            p = p * ax
            p = p * ax
            return p + 1.0
        if ax < 2.0:
            p = a * ax
            p = p - a5
            p = p * ax
            p = p + a8
            p = p * ax
            return p - a4
        return 0.0
    if kid == KID_MIT:
        if ax < 1.0:
            p = M1 * ax
            p = p - 2.0
            p = p * ax
            p = p * ax
            return p + M0
        if ax < 2.0:
            p = N3 * ax
            p = p + 2.0
            p = p * ax
            p = p - N1
            p = p * ax
            return p + N0
        return 0.0
    if kid == KID_LAN:
        if ax >= 3.0:
            return 0.0
        if x == 0.0:
            return 1.0
        s1 = sinpi(x)
        s2 = sinpi(x * THIRD)
        num = s2 * s1
        px = x * PI
        den = px * (px * THIRD)
        return num / den
    raise ValueError(kid)


def axis_weights(S, D, filt):
    """-> (n, nmax, identity, x0[D], W[D, n] int) exactly like rs_axis_weights."""
    kid, wide = FKID[filt], FWIDE[filt]
    if filt == RS_ADAPTIVE and not S > D:
        kid = KID_CUB75
    fant = kid == KID_FANT
    scale = S / D
    fs = max(1.0, scale) if wide else 1.0
    R = SUP[kid] * fs
    B = int(2 * R) + 4 if not fant else int(scale) + 3
    starts, taps = [], []
    for i in range(D):
        raw = []
        total = 0.0
        if not fant:
            center = ((2 * i + 1) * S) / (2 * D)
            jlo = math.floor(center - R)
            jhi = math.ceil(center + R)
            jhi = min(jhi, jlo + B - 1)
            for j in range(jlo, jhi + 1):
                d = (float(j) + 0.5) - center
                w = kern(kid, d / fs)
                raw.append(w)
                total += w
        else:
            lo = (i * S) / D
            hi = ((i + 1) * S) / D
            jlo = math.floor(lo)
            jhi = math.ceil(hi) - 1
            jhi = min(jhi, jlo + B - 1)
            for j in range(jlo, jhi + 1):
                w = min(float(j) + 1.0, hi) - max(float(j), lo)
                raw.append(w)
                total += w
        base = max(jlo, 0)
        top = min(jhi, S - 1)
        wf = [0.0] * (top - base + 1)
        for k, w in enumerate(raw):
            idx = min(max(jlo + k, 0), S - 1) - base
            wf[idx] += w
        if total == 0.0:
            total = 1.0
        q = []
        for w in wf:
            v = math.floor(w / total * 16384.0 + 0.5)
            q.append(min(max(v, -32767), 32767))
        am = max(range(len(q)), key=lambda k: (q[k], -k))
        q[am] += WONE - sum(q)
        f = next((k for k in range(len(q)) if q[k] != 0), len(q))
        if f == len(q):
            f, l = 0, 0
        else:
            l = max(k for k in range(len(q)) if q[k] != 0)
        starts.append(base + f)
        taps.append(q[f:l + 1])
    nmax = max(len(t) for t in taps)
    n = max(2, (nmax + 1) & ~1)
    x0 = np.zeros(D, np.int64)
    W = np.zeros((D, n), np.int64)
    for i in range(D):
        s = starts[i]
        x = s
        if x + n > S:
            x = max(0, S - n)
        off = s - x
        x0[i] = x
        W[i, off:off + len(taps[i])] = taps[i]
    ident = (S == D and nmax == 1)
    return n, nmax, ident, x0, W


# ---------------------------------------------------------------- gamma tables
def _s2l(x):
    return x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4


def _l2s(y):
    return 12.92 * y if y <= 0.0031308 else 1.055 * y ** (1 / 2.4) - 0.055


LIN16 = np.array([math.floor(16320 * _s2l(v / 255) + 0.5) for v in range(256)], np.int64)
SRGB8 = np.array([math.floor(255 * _l2s(i / 4095) + 0.5) for i in range(4096)], np.int64)


# ---------------------------------------------------------------- pixel helpers
def split(img):
    """uint32 (h, w) -> int64 (h, w, 4) channels b, g, r, a"""
    b = np.ascontiguousarray(img).view(np.uint8).reshape(img.shape + (4,))
    return b.astype(np.int64)


def join(ch):
    return np.ascontiguousarray(np.clip(ch, 0, 255).astype(np.uint8)).view(np.uint32).reshape(ch.shape[:2])


def premul8(img):
    c = split(img)
    a = c[..., 3:4]
    out = c.copy()
    out[..., :3] = ((c[..., :3] * a + 128) * 257) >> 16
    return join(out)


RCP = np.array([0] + [-(-(255 << 24) // a) for a in range(1, 256)], np.int64)


def unpremul8(img):
    c = split(img)
    a = c[..., 3]
    out = c.copy()
    r = RCP[a][..., None]
    v = np.minimum(255, (c[..., :3] * r + (1 << 23)) >> 24)
    out[..., :3] = v
    out[a == 0] = 0
    out[a == 255] = c[a == 255]
    return join(out)


def convert8(img, sfmt, dfmt):
    if sfmt == dfmt:
        return img.copy()
    return premul8(img) if sfmt == FMT_ARGB else unpremul8(img)


def to_work(img, fmt, gamma):
    c = split(img)
    a = c[..., 3]
    P = np.zeros(c.shape, np.int64)
    P[..., 3] = a << 6
    if not gamma:
        if fmt == FMT_PARGB:
            return c << 6
        t = c[..., :3] * a[..., None]
        P[..., :3] = ((t + 2) * 16448) >> 16
        return P
    if fmt == FMT_PARGB:
        am = np.maximum(a, 1)[..., None]
        s = np.minimum(255, (c[..., :3] * 255 + (am >> 1)) // am)
        s[a == 0] = 0
    else:
        s = c[..., :3]
    lin = LIN16[s]
    P[..., :3] = (lin * a[..., None] * 257 + 32768) >> 16
    return P


def from_work(P, fmt, gamma):
    """Pa = raw alpha (may overshoot 16320); colours are clamped to 0 .. Pa, the straight
    colour is Pc / Pa (ratio preserved), the alpha is clamped to 0 .. 16320."""
    Pr = P[..., 3]
    Pa = np.maximum(Pr, 0)
    Pc = np.maximum(np.minimum(P[..., :3], Pr[..., None]), 0)
    a8 = (np.minimum(Pa, PONE) + 32) >> 6
    out = np.zeros(P.shape, np.int64)
    out[..., 3] = a8
    if not gamma:
        num = Pc.astype(np.float32) * np.float32(255.0)
        den = np.maximum(Pa, 1).astype(np.float32)[..., None]
        straight = np.rint(num / den).astype(np.int64)
        if fmt == FMT_PARGB:
            out[..., :3] = np.where((Pa > PONE)[..., None], straight, (Pc + 32) >> 6)
        else:
            out[..., :3] = straight
        return join(out)
    pa = np.maximum(Pa, 1)[..., None]
    idx = np.minimum((Pc * 8190 + pa) // (2 * pa), 4095)
    s = SRGB8[idx]
    if fmt == FMT_PARGB:
        s = ((s * a8[..., None] + 128) * 257) >> 16
    out[..., :3] = s
    out[Pa == 0] = 0
    return join(out)


def sat16(x):
    return np.clip(x, -32768, 32767)


def ref_resize(img, sfmt, W, H, dfmt, filt, flags):
    h, w = img.shape
    if filt == RS_NEAREST:
        if (w, h) == (W, H):
            return convert8(img, sfmt, dfmt)
        xs = ((2 * np.arange(W, dtype=np.int64) + 1) * w) // (2 * W)
        ys = ((2 * np.arange(H, dtype=np.int64) + 1) * h) // (2 * H)
        return convert8(img[ys][:, xs], sfmt, dfmt)
    nh, _, idh, x0, WH = axis_weights(w, W, filt)
    nv, _, idv, y0, WV = axis_weights(h, H, filt)
    if idh and idv:
        return convert8(img, sfmt, dfmt)
    gamma = bool(flags & RS_GAMMA)
    P = to_work(img, sfmt, gamma)                       # (h, w, 4)
    if idh:
        Hs = P
    else:
        Pp = np.concatenate([P, np.zeros((h, nh + 2, 4), np.int64)], axis=1)
        acc = np.zeros((h, W, 4), np.int64)
        for k in range(nh):
            acc += WH[:, k][None, :, None] * Pp[:, x0 + k, :]
        Hs = sat16((acc + 8192) >> 14)
    acc = np.zeros((H, W, 4), np.int64)
    for j in range(nv):
        rows = np.minimum(y0 + j, h - 1)
        acc += WV[:, j][:, None, None] * Hs[rows, :, :]
    V = sat16((acc + 8192) >> 14)
    return from_work(V, dfmt, gamma)


def ref_mip(img, sfmt, x0=0, y0=0, x1=None, y1=None, prev=None):
    """mip_down: 2x2 box in premultiplied space, clamp to edge; dst FMT_PARGB."""
    h, w = img.shape
    W, H = (w + 1) // 2, (h + 1) // 2
    c = split(img)
    cols0 = np.minimum(2 * np.arange(W), w - 1)
    cols1 = np.minimum(2 * np.arange(W) + 1, w - 1)
    rows0 = np.minimum(2 * np.arange(H), h - 1)
    rows1 = np.minimum(2 * np.arange(H) + 1, h - 1)
    quad = [c[rows0][:, cols0], c[rows0][:, cols1], c[rows1][:, cols0], c[rows1][:, cols1]]
    out = np.zeros((H, W, 4), np.int64)
    if sfmt == FMT_PARGB:
        s = sum(quad)
        out = (s + 2) >> 2
    else:
        sa = sum(q[..., 3] for q in quad)
        sca = sum(q[..., :3] * q[..., 3:4] for q in quad)
        t = (sca + 2) >> 2
        out[..., :3] = ((t + 128) * 257) >> 16
        out[..., 3] = (sa + 2) >> 2
        out[..., :3] = np.minimum(out[..., :3], out[..., 3:4])
    return join(out)


# ---------------------------------------------------------------- file I/O
RAW_MAGIC = 0x31525352


def write_raw(path, arr, fmt):
    arr = np.ascontiguousarray(arr)
    h, w = arr.shape
    with open(path, "wb") as f:
        f.write(struct.pack("<4I", RAW_MAGIC, w, h, fmt))
        f.write(arr.tobytes())


def read_raw(path):
    d = Path(path).read_bytes()
    magic, w, h, fmt = struct.unpack_from("<4I", d)
    assert magic == RAW_MAGIC, path
    dt = np.uint8 if fmt == FMT_A8 else np.uint32
    return np.frombuffer(d, dt, count=w * h, offset=16).reshape(h, w), fmt


def read_weights(path):
    d = Path(path).read_bytes()
    magic, n, rb, nmax, ident, D = struct.unpack_from("<6I", d)
    assert magic == 0x31575352
    x0, W = [], []
    for i in range(D):
        o = 24 + i * rb
        x0.append(struct.unpack_from("<i", d, o)[0])
        pairs = np.frombuffer(d, np.int16, count=(rb - 16) // 2, offset=o + 16).reshape(-1, 8)
        W.append(np.stack([pairs[:, 0], pairs[:, 1]], 1).reshape(-1).astype(np.int64))
        assert (pairs[:, 0::2] == pairs[:, [0]]).all() and (pairs[:, 1::2] == pairs[:, [1]]).all()
    return n, nmax, ident, np.array(x0), np.array(W)


# ---------------------------------------------------------------- runner
class Runner:
    def __init__(self, target, out, name="rscheck"):
        self.target = target
        self.out = out
        self.work = out / name / target
        if self.work.exists():
            shutil.rmtree(self.work)
        self.work.mkdir(parents=True)
        self.lines = []

    def path(self, name):
        return self.work / name

    def rel(self, p):
        p = Path(p).resolve()
        try:
            return str(p.relative_to(ROOT)).replace("\\", "/")
        except ValueError:      # outside this checkout (jfgui_REF of another one): absolute
            s = str(p)
            return s.replace("\\", "/") if self.target == "win" else "/mnt/" + s[0].lower() + s[2:].replace("\\", "/")

    def add(self, *tokens):
        self.lines.append(" ".join(str(t) for t in tokens))

    def run(self, extra=()):
        ops = self.work / "ops.txt"
        ops.write_text("\n".join(self.lines) + "\n", encoding="utf-8")
        shot = self.rel(self.work / "shot.png")
        if self.target == "win":
            cmd = [str(self.out / "win" / "rstest.exe")]
        else:
            exe = self.rel(self.out / "linux" / "rstest")
            cmd = ["wsl.exe", "-d", WSL_DISTRO, "--cd", str(ROOT), "-e", exe if exe.startswith("/") else "./" + exe]
        cmd += ["--shot", shot, "--ops", self.rel(ops)] + list(extra)
        res = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        out = [l for l in res.stdout.splitlines() if l.startswith(("ok ", "err "))]
        if len(out) != len(self.lines):
            print(res.stdout[-3000:], res.stderr[-3000:])
            raise SystemExit(f"[{self.target}] rstest printed {len(out)} results for "
                             f"{len(self.lines)} ops (exit {res.returncode})")
        self.lines = []
        return out


def us_of(line):
    return int(line.split("us=")[1]) if line.startswith("ok ") else None


def rand_img(rng, w, h, fmt, transparent=0.1):
    c = rng.integers(0, 256, (h, w, 4), dtype=np.int64)
    c[..., 3] = np.where(rng.random((h, w)) < 0.5, 255, c[..., 3])
    c[..., 3] = np.where(rng.random((h, w)) < transparent, 0, c[..., 3])
    img = join(c)
    if fmt == FMT_PARGB:
        img = premul8(img)
    return img


# ---------------------------------------------------------------- tests
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/g2")
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    fails = 0
    for t in targets:
        fails += check_target(t, out, a)
    print("resample_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


def check_target(target, out, a):
    rng = np.random.default_rng(1234)
    R = Runner(target, out)
    fails = 0
    cases = []            # (kind, info, expected array, output path, tolerance)

    # --- weight tables
    wcases = [(7, 3), (3, 7), (100, 37), (37, 100), (1, 5), (5, 1), (13, 13), (65535, 3),
              (3, 700), (4000, 2000), (2, 65535)]
    for S, D in wcases:
        for f in range(1, 8):
            if D > 5000 and f in (RS_LANCZOS3, RS_ADAPTIVE):
                pass
            p = R.path(f"w_{S}_{D}_{f}.bin")
            R.add("weights", S, D, f, R.rel(p))
            cases.append(("weights", (S, D, f), None, p, 0))

    # --- resize matrix
    sizes = [((1, 1), (1, 1)), ((1, 1), (5, 3)), ((7, 5), (1, 1)), ((1, 9), (4, 3)),
             ((9, 1), (3, 4)), ((1, 12), (1, 5)), ((12, 1), (5, 1)), ((17, 13), (5, 7)),
             ((8, 8), (23, 19)), ((31, 29), (31, 29)), ((40, 30), (97, 11)),
             ((64, 48), (32, 24)), ((33, 21), (70, 9))]
    if not a.quick:
        sizes += [((300, 200), (97, 61)), ((40, 30), (173, 111)), ((5, 200), (3, 57))]
    for si, ((w, h), (W, H)) in enumerate(sizes):
        for sfmt in (FMT_ARGB, FMT_PARGB):
            img = rand_img(rng, w, h, sfmt)
            src = R.path(f"s{si}_{w}x{h}_{sfmt}.raw")
            write_raw(src, img, sfmt)
            for filt in range(8):
                for dfmt in (FMT_ARGB, FMT_PARGB):
                    for flags in (0, RS_GAMMA):
                        if filt == RS_NEAREST and flags:
                            continue
                        exp = ref_resize(img, sfmt, W, H, dfmt, filt, flags)
                        p = R.path(f"r_{w}x{h}_{W}x{H}_{sfmt}{dfmt}_{filt}_{flags}.raw")
                        R.add("resize", R.rel(src), R.rel(p), W, H, filt, flags, dfmt, 1, 1, 0)
                        cases.append(("resize", f"{w}x{h}->{W}x{H} {FILTER_NAMES[filt]} "
                                      f"s{sfmt} d{dfmt} g{flags}", exp, p, 2 if flags else 1))

    # --- identity exactness (same size, same format): every interpolating filter
    for sfmt in (FMT_ARGB, FMT_PARGB):
        img = rand_img(rng, 23, 17, sfmt, 0.3)
        src = R.path(f"id_{sfmt}.raw")
        write_raw(src, img, sfmt)
        for filt in (0, 1, 2, 3, 5, 6, 7):
            for flags in (0, RS_GAMMA):
                p = R.path(f"id_{sfmt}_{filt}_{flags}.raw")
                R.add("resize", R.rel(src), R.rel(p), 23, 17, filt, flags, sfmt, 1, 1, 0)
                cases.append(("exact", f"identity {FILTER_NAMES[filt]} s{sfmt} g{flags}", img, p, 0))
    # nearest integer upscale = replication; Fant 2x downscale = 2x2 box (premultiplied)
    img = rand_img(rng, 11, 7, FMT_ARGB, 0.3)
    src = R.path("int_a.raw")
    write_raw(src, img, FMT_ARGB)
    p = R.path("int_near.raw")
    R.add("resize", R.rel(src), R.rel(p), 33, 14, RS_NEAREST, 0, FMT_ARGB, 1, 1, 0)
    cases.append(("exact", "nearest 3x/2x replication", np.repeat(np.repeat(img, 2, 0), 3, 1), p, 0))
    imgp = rand_img(rng, 16, 12, FMT_PARGB, 0.3)
    src = R.path("int_p.raw")
    write_raw(src, imgp, FMT_PARGB)
    p = R.path("int_fant.raw")
    R.add("resize", R.rel(src), R.rel(p), 8, 6, RS_FANT, 0, FMT_PARGB, 1, 1, 0)
    cases.append(("exact", "fant 2x box", ref_mip(imgp, FMT_PARGB), p, 1))

    # --- transparent pixels must not bleed colour (straight in / out)
    c = np.zeros((24, 24, 4), np.int64)
    c[..., :3] = rng.integers(0, 256, (24, 24, 3))
    c[..., 3] = 0
    c[6:18, 6:18] = [40, 90, 200, 255]
    c[10:12, :, 3] = 0                                   # holes with random colours
    img = join(c)
    src = R.path("bleed.raw")
    write_raw(src, img, FMT_ARGB)
    for filt in range(1, 8):
        for (W, H) in ((11, 13), (53, 47)):
            for flags in (0, RS_GAMMA):
                p = R.path(f"bleed_{filt}_{W}_{flags}.raw")
                R.add("resize", R.rel(src), R.rel(p), W, H, filt, flags, FMT_ARGB, 1, 1, 0)
                cases.append(("bleed", f"{FILTER_NAMES[filt]} {W}x{H} g{flags}",
                              ref_resize(img, FMT_ARGB, W, H, FMT_ARGB, filt, flags), p,
                              2 if flags else 1))

    # --- huge sizes (65535 wide / tall), thread split determinism
    big = rand_img(rng, 65535, 2, FMT_ARGB, 0.05)
    src = R.path("wide.raw")
    write_raw(src, big, FMT_ARGB)
    for filt, (W, H) in ((RS_LANCZOS3, (1000, 3)), (RS_FANT, (7, 1)), (RS_BILINEAR, (65535, 5)),
                         (RS_BICUBIC, (3, 2))):
        p = R.path(f"wide_{filt}.raw")
        R.add("resize", R.rel(src), R.rel(p), W, H, filt, 0, FMT_PARGB, 1, 1, 0)
        cases.append(("resize", f"65535x2->{W}x{H} {FILTER_NAMES[filt]}",
                      ref_resize(big, FMT_ARGB, W, H, FMT_PARGB, filt, 0), p, 1))
    # more vertical taps than the ring budget holds (65535-wide rows: 128 ring rows): groups
    grp = rand_img(rng, 65535, 150, FMT_PARGB, 0.05)
    src = R.path("group.raw")
    write_raw(src, grp, FMT_PARGB)
    for filt, (W, H) in ((RS_LANCZOS3, (65000, 2)), (RS_BICUBIC, (65535, 1))):
        p = R.path(f"group_{filt}.raw")
        R.add("resize", R.rel(src), R.rel(p), W, H, filt, 0, FMT_ARGB, 1, 1, 0)
        cases.append(("resize", f"65535x150->{W}x{H} {FILTER_NAMES[filt]} (grouped taps)",
                      ref_resize(grp, FMT_PARGB, W, H, FMT_ARGB, filt, 0), p, 1))
    del grp
    tall = rand_img(rng, 2, 65535, FMT_PARGB, 0.05)
    src = R.path("tall.raw")
    write_raw(src, tall, FMT_PARGB)
    for filt, (W, H) in ((RS_LANCZOS3, (3, 1000)), (RS_FANT, (1, 7)), (RS_BICUBIC_SMOOTH, (2, 65535))):
        p = R.path(f"tall_{filt}.raw")
        R.add("resize", R.rel(src), R.rel(p), W, H, filt, 0, FMT_ARGB, 1, 1, 0)
        cases.append(("resize", f"2x65535->{W}x{H} {FILTER_NAMES[filt]}",
                      ref_resize(tall, FMT_PARGB, W, H, FMT_ARGB, filt, 0), p, 1))
    # row offsets beyond 4 GB: every surface of these ops gets a 1 MB + 4 byte stride
    BIG = (1 << 20) + 4
    tall = rand_img(rng, 3, 4200, FMT_ARGB, 0.05)
    src = R.path("bigstride.raw")
    write_raw(src, tall, FMT_ARGB)
    R.add("stride", BIG)
    cases.append(("none", "stride", None, None, 0))
    for filt, (W, H) in ((RS_LANCZOS3, (2, 4100)), (RS_NEAREST, (3, 4199)), (RS_BILINEAR, (5, 4201))):
        p = R.path(f"bigstride_{filt}.raw")
        R.add("resize", R.rel(src), R.rel(p), W, H, filt, 0, FMT_PARGB, 1, 1, 0)
        cases.append(("resize", f"stride > 4 GB 3x4200->{W}x{H} {FILTER_NAMES[filt]}",
                      ref_resize(tall, FMT_ARGB, W, H, FMT_PARGB, filt, 0), p, 1))
    p = R.path("bigstride_mip.raw")
    R.add("mip", R.rel(src), R.rel(p), 1, 1)
    cases.append(("resize", "stride > 4 GB mip", ref_mip(tall, FMT_ARGB), p, 0))
    R.add("stride", 0)
    cases.append(("none", "stride", None, None, 0))

    img = rand_img(rng, 257, 190, FMT_ARGB)
    src = R.path("split.raw")
    write_raw(src, img, FMT_ARGB)
    for filt in (RS_NEAREST, RS_LANCZOS3, RS_BILINEAR, RS_FANT):
        for W, H in ((83, 61), (400, 333)):
            p1 = R.path(f"split1_{filt}_{W}.raw")
            p4 = R.path(f"split4_{filt}_{W}.raw")
            R.add("resize", R.rel(src), R.rel(p1), W, H, filt, 0, FMT_ARGB, 1, 1, 0)
            R.add("resize", R.rel(src), R.rel(p4), W, H, filt, 0, FMT_ARGB, 1, 4, 0)
            exp = ref_resize(img, FMT_ARGB, W, H, FMT_ARGB, filt, 0)
            cases.append(("resize", f"split1 {FILTER_NAMES[filt]} {W}x{H}", exp, p1, 1))
            cases.append(("resize", f"split4 {FILTER_NAMES[filt]} {W}x{H}", exp, p4, 1))

    # --- mip
    mip_cases = []
    for (w, h) in ((1, 1), (2, 2), (3, 5), (17, 9), (64, 33), (129, 257)):
        for sfmt in (FMT_ARGB, FMT_PARGB):
            img = rand_img(rng, w, h, sfmt, 0.2)
            src = R.path(f"m_{w}x{h}_{sfmt}.raw")
            write_raw(src, img, sfmt)
            p = R.path(f"mo_{w}x{h}_{sfmt}.raw")
            R.add("mip", R.rel(src), R.rel(p), 1, 1)
            cases.append(("resize", f"mip {w}x{h} s{sfmt}", ref_mip(img, sfmt), p, 0))
            p = R.path(f"mc_{w}x{h}_{sfmt}_")
            R.add("mipchain", R.rel(src), R.rel(p), 0)
            lv, cur, cf = 1, img, sfmt
            while cur.shape != (1, 1):
                cur = ref_mip(cur, cf)
                cf = FMT_PARGB
                cases.append(("resize", f"mipchain {w}x{h} s{sfmt} level {lv}", cur,
                              Path(str(p) + f"{lv}.raw"), 0))
                lv += 1
    # incremental: base image, then a patch in a dirty rectangle
    for (w, h), rect in (((64, 33), (5, 3, 20, 17)), ((129, 257), (0, 0, 129, 1)),
                         ((129, 257), (128, 256, 129, 257)), ((17, 9), (3, 4, 4, 5))):
        for sfmt in (FMT_ARGB, FMT_PARGB):
            base = rand_img(rng, w, h, sfmt, 0.2)
            patch = rand_img(rng, w, h, sfmt, 0.2)
            b = R.path(f"mi_b_{w}_{sfmt}_{rect[0]}.raw")
            pa = R.path(f"mi_p_{w}_{sfmt}_{rect[0]}.raw")
            write_raw(b, base, sfmt)
            write_raw(pa, patch, sfmt)
            mod = base.copy()
            x0, y0, x1, y1 = rect
            mod[y0:y1, x0:x1] = patch[y0:y1, x0:x1]
            p = R.path(f"mi_o_{w}_{sfmt}_{rect[0]}_")
            R.add("mipinc", R.rel(b), R.rel(pa), R.rel(p), x0, y0, x1, y1)
            lv, cur, cf = 1, mod, sfmt
            while True:
                cur = ref_mip(cur, cf)
                cf = FMT_PARGB
                cases.append(("resize", f"mipinc {w}x{h} s{sfmt} {rect} level {lv}", cur,
                              Path(str(p) + f"{lv}.raw"), 0))
                lv += 1
                if cur.shape == (1, 1):
                    break

    results = R.run()
    fails += evaluate(target, results, cases)

    if a.bench:
        bench(R, rng)
    if not a.keep:
        shutil.rmtree(R.work, ignore_errors=True)
    return fails


def evaluate(target, results, cases):
    fails = 0
    worst = {}
    for res, (kind, info, exp, p, tol) in zip(results, cases):
        if not res.startswith("ok "):
            print(f"[{target}] FAIL {kind} {info}: {res}")
            fails += 1
            continue
        if kind == "none":
            continue
        if kind == "weights":
            S, D, f = info
            n, nmax, ident, x0, W = axis_weights(S, D, f)
            gn, gnmax, gid, gx0, gW = read_weights(p)
            ok = (n == gn and nmax == gnmax and int(ident) == gid and (x0 == gx0).all()
                  and (W == gW).all())
            if not ok:
                bad = np.nonzero((x0 != gx0) | (W != gW).any(1))[0][:3]
                print(f"[{target}] FAIL weights S={S} D={D} {FILTER_NAMES[f]}: n {n}/{gn} "
                      f"nmax {nmax}/{gnmax} id {ident}/{gid} rows {bad} "
                      f"exp {[(x0[i], W[i].tolist()) for i in bad]} got {[(gx0[i], gW[i].tolist()) for i in bad]}")
                fails += 1
            continue
        if not Path(p).exists():
            print(f"[{target}] FAIL {kind} {info}: no output {p}")
            fails += 1
            continue
        got, _ = read_raw(p)
        if got.shape != exp.shape:
            print(f"[{target}] FAIL {kind} {info}: shape {got.shape} != {exp.shape}")
            fails += 1
            continue
        d = np.abs(split(got) - split(exp))
        if kind == "bleed":
            # colour of every visible pixel must stay within the colours of the opaque block
            gc = split(got)
            vis = gc[..., 3] >= 8
            dev = np.abs(gc[..., :3] - np.array([40, 90, 200]))[vis]
            if dev.size and dev.max() > 2:
                print(f"[{target}] FAIL bleed {info}: colour deviates by {dev.max()}")
                fails += 1
                continue
        m = int(d.max()) if d.size else 0
        worst[kind] = max(worst.get(kind, 0), m)
        if m > tol:
            yx = np.unravel_index(np.argmax(d.max(-1)), d.shape[:2])
            print(f"[{target}] FAIL {kind} {info}: max |d| = {m} at {yx}: got "
                  f"{split(got)[yx].tolist()} exp {split(exp)[yx].tolist()}")
            fails += 1
    print(f"[{target}] {len(cases)} cases, {fails} failed; max |d| per kind: {worst}")
    return fails


def bench(R, rng):
    img = rand_img(rng, 4000, 3000, FMT_ARGB, 0.02)
    src = R.path("bench.raw")
    write_raw(src, img, FMT_ARGB)
    names = []
    for filt in range(8):
        R.add("resize", R.rel(src), R.rel(R.path("bench_o.raw")), 2000, 1500, filt, 0, FMT_ARGB, 3, 1, 0)
        names.append(f"resize 4000x3000->2000x1500 {FILTER_NAMES[filt]}")
    for filt in (RS_BILINEAR, RS_BICUBIC, RS_LANCZOS3):
        R.add("resize", R.rel(src), R.rel(R.path("bench_o.raw")), 6000, 4500, filt, 0, FMT_ARGB, 1, 1, 0)
        names.append(f"resize 4000x3000->6000x4500 {FILTER_NAMES[filt]}")
    R.add("resize", R.rel(src), R.rel(R.path("bench_o.raw")), 2000, 1500, RS_LANCZOS3, RS_GAMMA, FMT_ARGB, 1, 1, 0)
    names.append("resize 4000x3000->2000x1500 lanczos3 gamma")
    R.add("resize", R.rel(src), R.rel(R.path("bench_o.raw")), 2000, 1500, RS_LANCZOS3, 0, FMT_ARGB, 3, 8, 0)
    names.append("resize 4000x3000->2000x1500 lanczos3, 8 bands on the pool")
    R.add("mip", R.rel(src), R.rel(R.path("bench_m.raw")), 5, 1)
    names.append("mip_down 4000x3000 straight -> 2000x1500")
    res = R.run()
    for n, r in zip(names, res):
        us = us_of(r)
        print(f"[{R.target}] bench {n}: " + (f"{us / 1000:.1f} ms" if us is not None else r))


if __name__ == "__main__":
    sys.exit(main())
