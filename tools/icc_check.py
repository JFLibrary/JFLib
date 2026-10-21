#!/usr/bin/env python3
"""icc_check.py -- colour management checker (agent I1) for img/icc_xform.s.

Runs codectest batch ops 320..326 (apps/codectest/src/icc_ops.s) on Windows and/or Linux
(WSL) and compares with
  * a numpy float64 reference of the ICC maths (independent parser; matrix/TRC with curv
    identity / gamma / table and para 0..4, gray kTRC, lut8 / lut16 / mAB / mBA with
    tetrahedral CLUT interpolation, XYZ and Lab PCS in their encodings, intents incl.
    absolute (media white) and black point compensation) -- exact inverse TRCs by bisection;
  * a numpy model of our 33^3 3D-LUT path (grid nodes (k/32)^2, f32 tetrahedral) for LUT
    profiles and ICF_LUT;
  * LittleCMS 2 via ctypes inside WSL when liblcms2 is installed (nothing is installed).
Tolerances (per channel, 8 bit): matrix path vs reference +-1, 3D-LUT path vs its model
+-1, 3D-LUT path vs the exact reference and vs LittleCMS: reported (limits in LUT_TOL).
Suites: builtin, matrix, gray, lut, ident, rows, errors, fuzz, lcms, speed, monitor.

    python tools/icc_check.py [--target win|linux|both] [--out build/i1] [--only s1,s2]
"""

import random
import struct
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402

OP_XFORM, OP_INPLACE, OP_BUILTIN, OP_DESC, OP_BENCH, OP_MONITOR, OP_BUILD = range(320, 327)
ICF_BPC, ICF_NOIDENT, ICF_LUT = 1, 2, 4
ICX_IDENTITY, ICX_MATRIX, ICX_LUT = 0, 1, 2
D50 = np.array([0.9642, 1.0, 0.8249])
LUT_P99, LUT_MAX = 2, 32   # A2B sources vs the exact pipeline / LittleCMS: 99th percentile
                           # and maximum of |diff| (grid resampling of the source LUT)
B2A_P99 = 8                # B2A destinations: the destination LUT is resampled too (double
                           # interpolation; worst near black and at gamut borders)
LCMS_TOL = 2         # matrix/TRC profiles vs LittleCMS (its 8-bit optimised matrix-shaper path
                     # is off by up to 2 near black; our float path is within 1 of float64)
LCMS_GRAY_TOL = 6    # RGB -> gray vs LittleCMS: lcms resamples into a CLUT (steep gamma 2.2
                     # near black); ours is within 1 of the float64 reference
BUILTIN_NAMES = ["sRGB IEC61966-2.1 compatible", "A98 RGB compatible",
                 "Display P3 compatible", "ROMM RGB compatible", "Gray Gamma 2.2"]
PARA_N = [1, 3, 4, 5, 7]


# ================================================================== profile writer

def s15(v):
    return struct.pack(">i", int(round(v * 65536)))


def el_xyz(v):
    return b"XYZ \0\0\0\0" + b"".join(s15(x) for x in v)


def el_curv_id():
    return b"curv\0\0\0\0" + struct.pack(">I", 0)


def el_curv_gamma(g):
    return b"curv\0\0\0\0" + struct.pack(">IH", 1, int(round(g * 256)))


def el_curv_table(t):
    t = np.asarray(t, dtype=np.int64).clip(0, 65535)
    return b"curv\0\0\0\0" + struct.pack(">I", len(t)) + t.astype(">u2").tobytes()


def el_para(t, p):
    return b"para\0\0\0\0" + struct.pack(">HH", t, 0) + b"".join(s15(x) for x in p[:PARA_N[t]])


def el_mluc(s):
    u = s.encode("utf-16-be")
    return b"mluc\0\0\0\0" + struct.pack(">II", 1, 12) + b"enUS" + struct.pack(">II", len(u), 28) + u


def el_desc(s):
    a = s.encode("ascii") + b"\0"
    return b"desc\0\0\0\0" + struct.pack(">I", len(a)) + a + b"\0" * 8 + b"\0" * 70


def el_sf32(m):
    return b"sf32\0\0\0\0" + b"".join(s15(x) for x in np.ravel(m))


def pad4(b):
    return b + b"\0" * (-len(b) % 4)


def profile(tags, cs=b"RGB ", pcs=b"XYZ ", cls=b"mntr", ver=0x04300000):
    """tags: list of (sig, element bytes); equal elements are shared."""
    n = len(tags)
    off = 132 + 12 * n
    table, data, seen = [], b"", {}
    for sig, el in tags:
        if el in seen:
            o = seen[el]
        else:
            o = off + len(data)
            seen[el] = o
            data += pad4(el)
        table.append(struct.pack(">4sII", sig, o, len(el)))
    total = off + len(data)
    hdr = (struct.pack(">I", total) + b"\0" * 4 + struct.pack(">I", ver) + cls + cs + pcs +
           b"\0" * 12 + b"acsp" + b"\0" * 24 + struct.pack(">I", 0) +
           b"".join(s15(x) for x in D50) + b"\0" * 48)
    assert len(hdr) == 128
    return hdr + struct.pack(">I", n) + b"".join(table) + data


def el_lut16(clut, grid, intab=None, outtab=None, mat=None, n=2, m=2, nin=3, nout=3):
    mat = np.eye(3) if mat is None else mat
    if intab is None:
        intab = np.tile(np.linspace(0, 65535, n).round(), (nin, 1))
    if outtab is None:
        outtab = np.tile(np.linspace(0, 65535, m).round(), (nout, 1))
    return (b"mft2\0\0\0\0" + bytes([nin, nout, grid, 0]) + b"".join(s15(x) for x in np.ravel(mat)) +
            struct.pack(">HH", intab.shape[1], outtab.shape[1]) +
            np.asarray(intab, np.int64).clip(0, 65535).astype(">u2").tobytes() +
            np.asarray(clut, np.int64).clip(0, 65535).astype(">u2").tobytes() +
            np.asarray(outtab, np.int64).clip(0, 65535).astype(">u2").tobytes())


def el_lut8(clut, grid, mat=None):
    mat = np.eye(3) if mat is None else mat
    ident = np.tile(np.arange(256), (3, 1))
    return (b"mft1\0\0\0\0" + bytes([3, 3, grid, 0]) + b"".join(s15(x) for x in np.ravel(mat)) +
            ident.astype(np.uint8).tobytes() + np.asarray(clut, np.int64).clip(0, 255).astype(np.uint8).tobytes() +
            ident.astype(np.uint8).tobytes())


def el_mab(kind, B, M=None, A=None, clut=None, mat=None):
    """kind b'mAB ' / b'mBA '; B/M/A = lists of 3 curve elements; clut = (grid3, prec, data);
    mat = 12 values (3x3 row-major + offsets)."""
    body = b""
    off = 32
    offs = {}

    def put(name, blob):
        nonlocal body
        offs[name] = off + len(body)
        body += pad4(blob)
    put("B", b"".join(pad4(c) for c in B))
    if mat is not None:
        put("mat", b"".join(s15(x) for x in mat))
    if M is not None:
        put("M", b"".join(pad4(c) for c in M))
    if clut is not None:
        g, prec, d = clut
        gb = bytes(list(g) + [0] * (16 - len(g))) + bytes([prec, 0, 0, 0])
        dt = ">u2" if prec == 2 else np.uint8
        put("clut", gb + np.asarray(d, np.int64).astype(dt).tobytes())
    if A is not None:
        put("A", b"".join(pad4(c) for c in A))
    hdr = kind + b"\0" * 4 + bytes([3, 3, 0, 0]) + struct.pack(
        ">IIIII", offs.get("B", 0), offs.get("mat", 0), offs.get("M", 0), offs.get("clut", 0),
        offs.get("A", 0))
    return hdr + body


# ================================================================== reference parser / model

def rd_s15(b, o):
    return struct.unpack_from(">i", b, o)[0] / 65536.0


def parse_curve(b, o, size):
    sig = b[o:o + 4]
    if sig == b"curv":
        n = struct.unpack_from(">I", b, o + 8)[0]
        if n == 0:
            return {"t": "id"}, 12
        if n == 1:
            return {"t": "para", "pt": 0, "p": [struct.unpack_from(">H", b, o + 12)[0] / 256.0] + [0] * 6}, 14
        tab = np.frombuffer(b, ">u2", n, o + 12).astype(np.float64)
        return {"t": "tab", "tab": tab}, 12 + 2 * n
    if sig == b"para":
        ft = struct.unpack_from(">H", b, o + 8)[0]
        p = [rd_s15(b, o + 12 + 4 * i) for i in range(PARA_N[ft])] + [0] * 7
        return {"t": "para", "pt": ft, "p": p[:7]}, 12 + 4 * PARA_N[ft]
    raise ValueError("curve type")


def pw(t, g):
    if g == 0:
        return np.ones_like(t)
    return np.where(t > 0, np.power(np.maximum(t, 1e-300), g), 0.0)


def cv_eval(cv, x):
    x = np.clip(np.nan_to_num(x, nan=0.0), 0.0, 1.0)
    if cv["t"] == "id":
        return x
    if cv["t"] == "tab":
        T = cv["tab"]
        n = len(T)
        pos = x * (n - 1)
        i = np.minimum(pos.astype(np.int64), n - 2)
        f = pos - i
        return (T[i] + (T[i + 1] - T[i]) * f) / 65535.0
    g, a, b_, c, d, e, f = cv["p"]
    pt = cv["pt"]
    if pt == 0:
        return pw(x, g)
    if pt == 1:
        return pw(a * x + b_, g)
    if pt == 2:
        return pw(a * x + b_, g) + c
    if pt == 3:
        return np.where(x >= d, pw(a * x + b_, g), c * x)
    return np.where(x >= d, pw(a * x + b_, g) + e, c * x + f)


def cv_inv(cv, y):
    """Smallest x in 0..1 with (running max of) cv(x) >= y (bisection)."""
    y = np.asarray(y, np.float64)
    lo, hi = np.zeros_like(y), np.ones_like(y)
    for _ in range(48):
        mid = (lo + hi) * 0.5
        below = cv_eval(cv, mid) < y
        lo = np.where(below, mid, lo)
        hi = np.where(below, hi, mid)
    return hi


class Lut:
    pass


def parse_lut(b, o, size, pcslab, direction):
    L = Lut()
    L.pcslab = pcslab
    sig = b[o:o + 4]
    L.mat = None
    L.flags = set()
    if sig in (b"mft2", b"mft1"):
        L.type = "lut16" if sig == b"mft2" else "lut8"
        g = b[o + 10]
        L.g = [g, g, g]
        if L.type == "lut16":
            n, m = struct.unpack_from(">HH", b, o + 48)
            L.prec, p = 2, o + 52
            dt = ">u2"
        else:
            n = m = 256
            L.prec, p = 1, o + 48
            dt = np.uint8
        sc = 65535.0 if L.prec == 2 else 255.0
        L.intab = np.frombuffer(b, dt, 3 * n, p).astype(np.float64).reshape(3, n) / sc
        p += 3 * n * L.prec
        L.clut = np.frombuffer(b, dt, g ** 3 * 3, p).astype(np.float64).reshape(g, g, g, 3) / sc
        p += g ** 3 * 3 * L.prec
        L.outtab = np.frombuffer(b, dt, 3 * m, p).astype(np.float64).reshape(3, m) / sc
        if direction == 1 and not pcslab:
            L.mat = np.array([rd_s15(b, o + 12 + 4 * i) for i in range(9)] + [0, 0, 0])
        return L
    L.type = "mab" if sig == b"mAB " else "mba"
    oB, oMat, oM, oC, oA = struct.unpack_from(">IIIII", b, o + 12)

    def curves(off):
        res = []
        for _ in range(3):
            c, used = parse_curve(b, o + off, size - off)
            res.append(c)
            off += (used + 3) & ~3
        return res
    L.B = curves(oB)
    L.M = curves(oM) if oM else None
    L.A = curves(oA) if oA else None
    if oMat:
        L.mat = np.array([rd_s15(b, o + oMat + 4 * i) for i in range(12)])
    L.clut = None
    if oC:
        g = list(b[o + oC:o + oC + 3])
        prec = b[o + oC + 16]
        dt = ">u2" if prec == 2 else np.uint8
        L.clut = np.frombuffer(b, dt, g[0] * g[1] * g[2] * 3, o + oC + 20).astype(np.float64).reshape(
            g[0], g[1], g[2], 3) / (65535.0 if prec == 2 else 255.0)
        L.g = g
    return L


def tab_interp(tab, x):
    n = len(tab)
    x = np.clip(x, 0, 1) * (n - 1)
    i = np.minimum(x.astype(np.int64), n - 2)
    f = x - i
    return tab[i] + (tab[i + 1] - tab[i]) * f


def clut_tetra(clut, v):
    g = clut.shape[:3]
    idx, fr = [], []
    for d in range(3):
        x = np.clip(v[d], 0, 1) * (g[d] - 1)
        i = np.minimum(x.astype(np.int64), g[d] - 2)
        idx.append(i)
        fr.append(x - i)
    fr = np.stack(fr)                              # (3, N)
    order = np.argsort(-fr, axis=0, kind="stable")
    fs = np.take_along_axis(fr, order, 0)
    pos = [idx[0].copy(), idx[1].copy(), idx[2].copy()]
    out = clut[pos[0], pos[1], pos[2]].T.copy()    # (3, N)
    prev = out.copy()
    for k in range(3):
        ax = order[k]
        for d in range(3):
            pos[d] = pos[d] + (ax == d)
        cur = clut[pos[0], pos[1], pos[2]].T
        out += (cur - prev) * fs[k]
        prev = cur
    return out


def mat_apply(mat, v, offs):
    m = mat[:9].reshape(3, 3)
    r = m @ v
    if offs:
        r = r + mat[9:12, None]
    return r


def lut_eval(L, v):
    v = np.array(v, np.float64)
    if L.type in ("lut16", "lut8"):
        if L.mat is not None:
            v = mat_apply(L.mat, v, False)
        v = np.stack([tab_interp(L.intab[c], v[c]) for c in range(3)])
        v = clut_tetra(L.clut, v)
        return np.stack([tab_interp(L.outtab[c], v[c]) for c in range(3)])
    if L.type == "mab":
        if L.A:
            v = np.stack([cv_eval(L.A[c], v[c]) for c in range(3)])
        if L.clut is not None:
            v = clut_tetra(L.clut, v)
        if L.M:
            v = np.stack([cv_eval(L.M[c], v[c]) for c in range(3)])
        if L.mat is not None:
            v = mat_apply(L.mat, v, True)
        return np.stack([cv_eval(L.B[c], v[c]) for c in range(3)])
    v = np.stack([cv_eval(L.B[c], v[c]) for c in range(3)])
    if L.mat is not None:
        v = mat_apply(L.mat, v, True)
    if L.M:
        v = np.stack([cv_eval(L.M[c], v[c]) for c in range(3)])
    if L.clut is not None:
        v = clut_tetra(L.clut, v)
    if L.A:
        v = np.stack([cv_eval(L.A[c], v[c]) for c in range(3)])
    return v


def lab_to_xyz(lab):
    L, a, b = lab
    fy = (L + 16) / 116
    fx = fy + a / 500
    fz = fy - b / 200

    def finv(t):
        return np.where(t > 6 / 29, t ** 3, (t - 4 / 29) * 108 / 841)
    return np.stack([D50[0] * finv(fx), finv(fy), D50[2] * finv(fz)])


def xyz_to_lab(xyz):
    def f(t):
        return np.where(t > 216 / 24389, np.cbrt(np.maximum(t, 0)), t * 841 / 108 + 4 / 29)
    fx, fy, fz = f(xyz[0] / D50[0]), f(xyz[1]), f(xyz[2] / D50[2])
    return np.stack([116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)])


def pcs_decode(L, v):
    if not L.pcslab:
        return v * (65535 / 32768)
    if L.type == "lut16":
        lab = np.stack([v[0] * 100.390625, v[1] * 255.99609375 - 128, v[2] * 255.99609375 - 128])
    else:
        lab = np.stack([v[0] * 100, v[1] * 255 - 128, v[2] * 255 - 128])
    return lab_to_xyz(lab)


def pcs_encode(L, xyz):
    if not L.pcslab:
        return xyz * (32768 / 65535)
    lab = xyz_to_lab(xyz)
    if L.type == "lut16":
        return np.stack([lab[0] * 65280 / 6553500, (lab[1] + 128) * 256 / 65535, (lab[2] + 128) * 256 / 65535])
    return np.stack([lab[0] / 100, (lab[1] + 128) / 255, (lab[2] + 128) / 255])


class Prof:
    """Reference model of one profile (mirrors pm_parse)."""

    def __init__(self, b):
        self.b = b
        self.size, = struct.unpack_from(">I", b, 0)
        self.ver = b[8]
        self.cls, self.cs, self.pcs = b[12:16], b[16:20], b[20:24]
        n, = struct.unpack_from(">I", b, 128)
        self.tags = {}
        for i in range(n):
            sig, o, s = struct.unpack_from(">4sII", b, 132 + 12 * i)
            self.tags.setdefault(sig, (o, s))
        self.wtpt = D50.copy()
        if not (self.ver < 4 and self.cls == b"mntr") and b"wtpt" in self.tags:
            o, s = self.tags[b"wtpt"]
            if b[o:o + 4] == b"XYZ ":
                self.wtpt = np.array([rd_s15(b, o + 8 + 4 * i) for i in range(3)])
        self.mat, self.cv = None, None
        if self.cs == b"GRAY":
            if b"kTRC" in self.tags:
                o, s = self.tags[b"kTRC"]
                c, _ = parse_curve(b, o, s)
                self.cv = [c, c, c]
                self.mat = np.array([[0, D50[0], 0], [0, 1, 0], [0, D50[2], 0]])
        else:
            try:
                cols = []
                for t in (b"rXYZ", b"gXYZ", b"bXYZ"):
                    o, s = self.tags[t]
                    cols.append([rd_s15(b, o + 8 + 4 * i) for i in range(3)])
                self.mat = np.array(cols).T
                self.cv = [parse_curve(b, *self.tags[t])[0] for t in (b"rTRC", b"gTRC", b"bTRC")]
            except (KeyError, ValueError, IndexError):
                self.mat = self.cv = None

    def lut_tag(self, pre, intent):
        i = 1 if intent == 3 else intent
        for sig in (pre + str(i).encode(), pre + b"0"):
            if sig in self.tags:
                return self.tags[sig]
        return None


def ref_transform(src, dst, intent, flags, px, lut_model=False):
    """Reference BGRA -> BGRA (alpha kept).  lut_model: model our 33^3 grid path."""
    if src == dst and not (flags & (ICF_NOIDENT | ICF_LUT)):
        return px.copy()                   # identical profiles: rows are copied
    S, D = Prof(src), Prof(dst)
    a, bb = np.ones(3), np.zeros(3)
    sl = None if S.cs == b"GRAY" else S.lut_tag(b"A2B", intent)
    dl = None if D.cs == b"GRAY" else D.lut_tag(b"B2A", intent)
    SL = parse_lut(src, sl[0], sl[1], S.pcs == b"Lab ", 0) if sl else None
    DL = parse_lut(dst, dl[0], dl[1], D.pcs == b"Lab ", 1) if dl else None
    if D.cs == b"GRAY":
        MD = np.array([[0, 1, 0]] * 3, np.float64)
    elif DL is None:
        MD = np.linalg.inv(D.mat)

    def src_xyz(v):
        if SL is not None:
            x = pcs_decode(SL, lut_eval(SL, v))
        else:
            x = S.mat @ np.stack([cv_eval(S.cv[c], v[c]) for c in range(3)])
        return x * a[:, None] + bb[:, None]

    def dst_dev(x, ext=False):
        if DL is not None:
            e = np.clip(pcs_encode(DL, x), 0, 1)
            return np.clip(lut_eval(DL, e), 0, 1)
        if ext:
            return MD @ x                  # grid of a matrix destination: linear, unclamped
        lin = np.clip(MD @ x, 0, 1)
        return np.stack([cv_inv(D.cv[c], lin[c]) for c in range(3)])
    if intent == 3:
        a = np.where(D.wtpt > 1e-6, S.wtpt / np.where(D.wtpt > 1e-6, D.wtpt, 1), 1.0)
    elif flags & ICF_BPC:
        bs = src_xyz(np.zeros((3, 1)))[:, 0]
        bd = np.zeros(3)
        if DL is not None:
            al = D.lut_tag(b"A2B", intent)
            if al:
                L2 = parse_lut(dst, al[0], al[1], D.pcs == b"Lab ", 0)
                bd = pcs_decode(L2, lut_eval(L2, np.zeros((3, 1))))[:, 0]
        t = bs - D50
        ok = np.abs(t) > 1e-9
        a = np.where(ok, (bd - D50) / np.where(ok, t, 1), 1.0)
        bb = np.where(ok, -D50 * (bd - bs) / np.where(ok, t, 1), 0.0)
    dev = np.stack([px[:, 2], px[:, 1], px[:, 0]]).astype(np.float64) / 255.0   # R, G, B
    if not lut_model:
        out = dst_dev(src_xyz(dev))
        rgb = np.rint(np.clip(out, 0, 1) * 255).astype(np.uint8)
    else:
        k = np.arange(33, dtype=np.float64)
        xs = (k * k) / 1024.0
        R, G, B = np.meshgrid(xs, xs, xs, indexing="ij")
        nodes = dst_dev(src_xyz(np.stack([R.ravel(), G.ravel(), B.ravel()])), ext=True).astype(np.float32)
        grid = nodes.reshape(3, 33, 33, 33)
        u = np.arange(256, dtype=np.float64)
        x1024 = (u / 255.0) * 1024.0
        kk = np.floor(np.sqrt(u / 255.0) * 32).astype(np.int64)
        kk = np.where(kk * kk > x1024, kk - 1, kk)
        kk = np.where((kk + 1) ** 2 <= x1024, kk + 1, kk)
        kk = np.minimum(kk, 31)
        frac = ((x1024 - kk * kk) / (2 * kk + 1)).astype(np.float32)
        iR, iG, iB = (kk[px[:, 2]], kk[px[:, 1]], kk[px[:, 0]])
        fR, fG, fB = (frac[px[:, 2]], frac[px[:, 1]], frac[px[:, 0]])

        def node(dr, dg, db):
            return grid[:, iR + dr, iG + dg, iB + db]       # (3, N) float32 R G B
        c0 = node(0, 0, 0)
        c3 = node(1, 1, 1)
        # the same decision tree as xf_row_lut: first / second vertex and weights
        rg, gb, rb = fR >= fG, fG >= fB, fR >= fB
        case = np.where(rg, np.where(gb, 1, np.where(rb, 2, 3)), np.where(rb, 4, np.where(gb, 5, 6)))
        v1 = {1: (1, 0, 0), 2: (1, 0, 0), 3: (0, 0, 1), 4: (0, 1, 0), 5: (0, 1, 0), 6: (0, 0, 1)}
        v2 = {1: (1, 1, 0), 2: (1, 0, 1), 3: (1, 0, 1), 4: (1, 1, 0), 5: (0, 1, 1), 6: (0, 1, 1)}
        w = {1: (fR, fG, fB), 2: (fR, fB, fG), 3: (fB, fR, fG), 4: (fG, fR, fB), 5: (fG, fB, fR),
             6: (fB, fG, fR)}
        res = np.zeros((3, len(px)), np.float32)
        linear = DL is None
        if linear:
            j = np.arange(4097, dtype=np.float64)
            outt = [np.rint(np.clip(cv_inv(D.cv[c], (j / 4096) ** 2), 0, 1) * 255) for c in range(3)]
        for cs_ in range(1, 7):
            m = case == cs_
            if not m.any():
                continue
            c1 = node(*v1[cs_])[:, m]
            c2 = node(*v2[cs_])[:, m]
            w1, w2, w3 = (x[m] for x in w[cs_])
            r = c0[:, m].copy()
            r = r + (c1 - c0[:, m]) * w1
            r = r + (c2 - c1) * w2
            r = r + (c3[:, m] - c2) * w3
            res[:, m] = r
        if linear:
            idx = np.rint(np.sqrt(np.clip(res, np.float32(0), np.float32(1))) * np.float32(4096)).astype(np.int64)
            rgb = np.stack([outt[c][idx[c]] for c in range(3)]).astype(np.uint8)
        else:
            rgb = np.clip(np.rint(res * np.float32(255)), 0, 255).astype(np.uint8)
    out = px.copy()
    out[:, 2], out[:, 1], out[:, 0] = rgb[0], rgb[1], rgb[2]
    return out


# ================================================================== test profiles

def rgb_matrix(prim, white):
    def xyz(x, y):
        return np.array([x / y, 1.0, (1 - x - y) / y])
    P = np.array([xyz(*c) for c in prim]).T
    w = xyz(*white)
    S = np.linalg.solve(P, w)
    M = P * S
    BR = np.array([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]])
    C = np.linalg.inv(BR) @ np.diag((BR @ D50) / (BR @ w)) @ BR
    return C @ M, w


SRGB_PRIM = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)]
WIDE_PRIM = [(0.70, 0.29), (0.17, 0.78), (0.13, 0.04)]
D65 = (0.3127, 0.3290)
SRGB_PARA = (3, [2.4, 1 / 1.055, 0.055 / 1.055, 1 / 12.92, 0.04045])


def mtx_profile(M, trcs, ver=0x04300000, wtpt=None, desc="test", v2desc=False):
    tags = [(b"desc", el_desc(desc) if v2desc else el_mluc(desc)), (b"wtpt", el_xyz(D50 if wtpt is None else wtpt))]
    for i, t in enumerate((b"rXYZ", b"gXYZ", b"bXYZ")):
        tags.append((t, el_xyz(M[:, i])))
    for t, e in zip((b"rTRC", b"gTRC", b"bTRC"), trcs):
        tags.append((t, e))
    return profile(tags, ver=ver)


def gray_profile(trc, desc="gray"):
    return profile([(b"desc", el_mluc(desc)), (b"wtpt", el_xyz(D50)), (b"kTRC", trc)], cs=b"GRAY")


def srgb_like_curve(kind, rng):
    if kind == "para3":
        return el_para(*SRGB_PARA)
    if kind == "gamma":
        return el_curv_gamma(rng.choice([1.0, 1.8, 2.2, 2.4]))
    if kind == "table":
        n = rng.choice([2, 17, 256, 1024, 4096])
        x = np.linspace(0, 1, n)
        return el_curv_table(np.rint(np.power(x, rng.uniform(1.6, 2.6)) * 65535))
    if kind == "id":
        return el_curv_id()
    t = int(kind[4:])
    g = rng.uniform(1.8, 2.6)
    a, b, d = 0.947, 0.053, 0.04045
    c = (a * d + b) ** g / d              # continuous at d (monotonic curve)
    p = {0: [g], 1: [g, 1.0, 0.0], 2: [g, 0.95, 0.05, 0.0], 3: [g, a, b, c, d],
         4: [g, a, b, c, d, 0.0, 0.0]}[t]
    return el_para(t, p)


def srgb_fwd(dev):
    """sRGB device (3, N) -> XYZ D50 (exact maths)."""
    M, _ = rgb_matrix(SRGB_PRIM, D65)
    lin = np.where(dev <= 0.04045, dev / 12.92, ((dev + 0.055) / 1.055) ** 2.4)
    return M @ lin


def srgb_inv(xyz):
    M, _ = rgb_matrix(SRGB_PRIM, D65)
    lin = np.clip(np.linalg.inv(M) @ xyz, 0, 1)
    return np.where(lin <= 0.0031308, lin * 12.92, 1.055 * lin ** (1 / 2.4) - 0.055)


def grid_pts(g):
    x = np.linspace(0, 1, g)
    R, G, B = np.meshgrid(x, x, x, indexing="ij")
    return np.stack([R.ravel(), G.ravel(), B.ravel()])


def lut_profiles():
    """(name, bytes) of RGB LUT profiles that all describe sRGB (so LittleCMS can compare)."""
    res = []
    M, _ = rgb_matrix(SRGB_PRIM, D65)
    base = [(b"desc", el_mluc("lut test")), (b"wtpt", el_xyz(D50))]
    # lut16 A2B0, PCS XYZ, grid 17
    g = 17
    xyz = srgb_fwd(grid_pts(g)) * (32768 / 65535)
    res.append(("a2b_lut16_xyz", profile(base + [(b"A2B0", el_lut16(np.rint(xyz.T * 65535), g))], cs=b"RGB ", cls=b"scnr")))
    # lut16 A2B0, PCS Lab (v2 16-bit encoding), with an input shaper
    lab = xyz_to_lab(srgb_fwd(grid_pts(g)))
    enc = np.stack([lab[0] * 65280 / 100, (lab[1] + 128) * 256, (lab[2] + 128) * 256])
    res.append(("a2b_lut16_lab", profile(base + [(b"A2B0", el_lut16(np.rint(enc.T), g, n=256))],
                                         pcs=b"Lab ", cls=b"scnr")))
    # lut8 A2B0, PCS Lab, grid 9
    g8 = 9
    lab = xyz_to_lab(srgb_fwd(grid_pts(g8)))
    enc8 = np.stack([lab[0] * 255 / 100, lab[1] + 128, lab[2] + 128])
    res.append(("a2b_lut8_lab", profile(base + [(b"A2B0", el_lut8(np.rint(enc8.T), g8))], pcs=b"Lab ", cls=b"scnr")))
    # mAB: A = sRGB TRC, CLUT = identity (grid 5, 16 bit), M = identity, matrix -> XYZ, B = identity
    g5 = 5
    ident = grid_pts(g5)
    mat = np.concatenate([(M * (32768 / 65535)).ravel(), [0, 0, 0]])
    trc = el_para(*SRGB_PARA)
    mab = el_mab(b"mAB ", [el_curv_id()] * 3, M=[el_curv_id()] * 3, A=[trc] * 3,
                 clut=((g5, g5, g5), 2, np.rint(ident.T * 65535).ravel()), mat=mat)
    res.append(("a2b_mab", profile(base + [(b"A2B0", mab)], cls=b"scnr")))
    # mAB without CLUT: B curves only after a matrix (M + matrix)
    mab2 = el_mab(b"mAB ", [el_curv_id()] * 3, M=[trc] * 3, mat=mat)
    res.append(("a2b_mab_nolut", profile(base + [(b"A2B0", mab2)], cls=b"scnr")))
    # destination LUTs: mBA (B id, matrix XYZ -> linear, M id, CLUT id, A = inverse TRC table)
    Mi = np.linalg.inv(M) * (65535 / 32768)
    inv_t = el_curv_table(np.rint(srgb_inv_curve(np.linspace(0, 1, 4096)) * 65535))
    mba = el_mab(b"mBA ", [el_curv_id()] * 3, M=[el_curv_id()] * 3, A=[inv_t] * 3,
                 clut=((g5, g5, g5), 2, np.rint(ident.T * 65535).ravel()),
                 mat=np.concatenate([Mi.ravel(), [0, 0, 0]]))
    fwd_mab = mab
    res.append(("b2a_mba", profile(base + [(b"A2B0", fwd_mab), (b"B2A0", mba)], cls=b"mntr")))
    # lut16 B2A0 PCS XYZ, grid 33 over encoded XYZ (0..1 -> 0..1.99997)
    g33 = 33
    pts = grid_pts(g33) * (65535 / 32768)
    devs = np.clip(srgb_inv(pts), 0, 1)
    res.append(("b2a_lut16_xyz", profile(base + [(b"A2B0", el_lut16(np.rint(xyz.T * 65535), g)),
                                                 (b"B2A0", el_lut16(np.rint(devs.T * 65535), g33))], cls=b"mntr")))
    # lut16 B2A0 with PCS Lab (v2 16-bit encoding), grid 33, plus its A2B0 (Lab)
    u = grid_pts(g33)
    labg = np.stack([u[0] * 65535 / 65280 * 100, u[1] * 65535 / 256 - 128, u[2] * 65535 / 256 - 128])
    devl = np.clip(srgb_inv(lab_to_xyz(labg)), 0, 1)
    res.append(("b2a_lut16_lab", profile(base + [(b"A2B0", el_lut16(np.rint(enc.T), g, n=256)),
                                                 (b"B2A0", el_lut16(np.rint(devl.T * 65535), g33))],
                                         pcs=b"Lab ", cls=b"mntr")))
    return res


def srgb_inv_curve(lin):
    return np.where(lin <= 0.0031308, lin * 12.92, 1.055 * np.power(lin, 1 / 2.4) - 0.055)


# ================================================================== records

def xrec(src, dst, intent, flags, px, op=OP_XFORM, arg=0):
    s = b"" if src is None else src
    d = b"" if dst is None else dst
    return struct.pack("<IIIII", len(s), len(d), intent, flags, len(px)) + s + d + px.tobytes()


def test_pixels(rng, n_rand=2048):
    lv = np.array([0, 1, 2, 5, 10, 20, 40, 64, 96, 128, 160, 192, 224, 250, 254, 255], np.uint8)
    B, G, R = np.meshgrid(lv, lv, lv, indexing="ij")
    grid = np.stack([B.ravel(), G.ravel(), R.ravel(), np.full(B.size, 255, np.uint8)], 1)
    ramp = np.stack([np.arange(256, dtype=np.uint8)] * 3 + [np.arange(256, dtype=np.uint8)[::-1]], 1)
    r = np.random.default_rng(rng.randrange(1 << 30))
    rnd = r.integers(0, 256, (n_rand, 4), dtype=np.uint8)
    return np.ascontiguousarray(np.concatenate([grid, ramp, rnd]).astype(np.uint8))


def cmp_px(out, ref, tol, stats=None):
    a = out.reshape(-1, 4).astype(np.int32)
    b = ref.reshape(-1, 4).astype(np.int32)
    if (a[:, 3] != b[:, 3]).any():
        return "alpha changed"
    d = np.abs(a[:, :3] - b[:, :3])
    if stats is not None:
        stats.append((d.max() if d.size else 0, float((d == 0).mean()) if d.size else 1.0))
    if d.size and d.max() > tol:
        i = int(np.argmax(d.max(1)))
        return f"max diff {d.max()} > {tol} at px {i}: got {a[i].tolist()} want {b[i].tolist()}"
    return None


def expect_px(px, ref, tol, kinds=None, stats=None):
    def chk(res, out):
        if res < 0:
            return f"error {it.err_name(res)}"
        if kinds is not None and res not in kinds:
            return f"kind {res} not in {kinds}"
        o = np.frombuffer(out, np.uint8).reshape(-1, 4)
        if len(o) != len(px):
            return f"got {len(o)} pixels"
        return cmp_px(o, ref, tol, stats)
    return chk


def expect_err(want):
    def chk(res, out):
        if res != want:
            return f"result {it.err_name(res)} want {it.err_name(want)}"
        return None
    return chk


def expect_stat(px, ref, p99, mx, kinds=None, stats=None):
    """LUT approximations: 99th percentile of the channel differences <= p99, max <= mx."""
    def chk(res, out):
        if res < 0:
            return f"error {it.err_name(res)}"
        if kinds is not None and res not in kinds:
            return f"kind {res} not in {kinds}"
        o = np.frombuffer(out, np.uint8).reshape(-1, 4).astype(np.int32)
        d = np.abs(o[:, :3] - ref.reshape(-1, 4)[:, :3].astype(np.int32))
        if (o[:, 3] != ref.reshape(-1, 4)[:, 3]).any():
            return "alpha changed"
        q = float(np.percentile(d, 99))
        if stats is not None:
            stats.append((int(d.max()), float((d == 0).mean()), q, float(d.mean())))
        if q > p99 or d.max() > mx:
            i = int(np.argmax(d.max(1)))
            return f"p99 {q} (<= {p99}) max {d.max()} (<= {mx}) at px {i}: got {o[i].tolist()} want {ref[i].tolist()}"
        return None
    return chk


def stat_note(name, stats):
    if not stats:
        return f"{name}: no data"
    mx = max(s[0] for s in stats)
    ex = sum(s[1] for s in stats) / len(stats)
    extra = ""
    if len(stats[0]) > 2:
        extra = f", worst p99 {max(s[2] for s in stats):.1f}, mean |diff| {sum(s[3] for s in stats) / len(stats):.3f}"
    return f"{name}: {len(stats)} transforms, max |diff| {mx}, exact {ex * 100:.2f} %{extra}"


# ================================================================== suites

def get_builtins(runner):
    res = runner.batch([it.Rec(OP_BUILTIN, i, b"", None, f"bi{i}") for i in range(5)])
    return [bytes(r[1]) for r in res]


def suite_builtin(runner, target, rng):
    s = it.Suite("builtin", target)
    recs = [it.Rec(OP_BUILTIN, 5, b"", expect_err(-1), "bad id")]
    bis = get_builtins(runner)
    std = {0: (SRGB_PRIM, D65), 1: ([(0.64, 0.33), (0.21, 0.71), (0.15, 0.06)], D65),
           2: ([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)], D65),
           3: ([(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001)], (0.3457, 0.3585))}
    for i, b in enumerate(bis):
        s.check(len(b) > 132 and b[36:40] == b"acsp" and struct.unpack_from(">I", b, 0)[0] == len(b),
                f"builtin {i}: bad header")
        p = Prof(b)
        if i in std:
            M, _ = rgb_matrix(*std[i])
            s.check(p.mat is not None and np.abs(p.mat - M).max() < 1e-4, f"builtin {i}: colorants off")
            s.check(np.abs(p.mat.sum(1) - D50).max() < 1e-4, f"builtin {i}: white is not D50")
        else:
            s.check(p.cs == b"GRAY" and p.cv is not None, "gray builtin")
        recs.append(it.Rec(OP_DESC, 256, b, (lambda want: lambda r, o: None if o[:-1].decode() == want
                                             else f"desc {o!r}")(BUILTIN_NAMES[i]), f"desc {i}"))
    # descriptions: v2 'desc', v4 mluc with German preferred, UTF-16 surrogates, truncation
    M, _ = rgb_matrix(SRGB_PRIM, D65)
    trc = [el_para(*SRGB_PARA)] * 3
    recs.append(it.Rec(OP_DESC, 64, mtx_profile(M, trc, desc="v2 Beschreibung", v2desc=True),
                       lambda r, o: None if o == b"v2 Beschreibung\0" else f"{o!r}", "desc v2"))
    de, en = "Grün\U0001F600".encode("utf-16-be"), "Hello".encode("utf-16-be")
    ml = (b"mluc\0\0\0\0" + struct.pack(">II", 2, 12) + b"enUS" + struct.pack(">II", len(en), 40) +
          b"deDE" + struct.pack(">II", len(de), 40 + len(en)) + en + de)
    p2 = profile([(b"desc", ml), (b"wtpt", el_xyz(D50))])
    recs.append(it.Rec(OP_DESC, 64, p2, lambda r, o: None if o.decode("utf-8") == "Grün\U0001F600\0"
                       else f"{o!r}", "desc mluc de + surrogate"))
    recs.append(it.Rec(OP_DESC, 4, p2, lambda r, o: None if o == b"Gr\0" else f"{o!r}", "desc cap 4"))
    recs.append(it.Rec(OP_DESC, 64, b"\0" * 100, lambda r, o: None if r == 0 and o == b"\0" else f"{r} {o!r}", "desc junk"))
    it.run_suite(runner, s, recs)
    return s, bis


def suite_matrix(runner, target, rng, bis):
    s = it.Suite("matrix", target)
    px = test_pixels(rng)
    recs, stats = [], []
    rgb = [b for b in bis[:4]]
    for i in range(4):
        for j in range(4):
            if i == j:
                continue
            for intent in (0, 1, 3):
                ref = ref_transform(rgb[i], rgb[j], intent, 0, px)
                recs.append(it.Rec(OP_XFORM, 0, xrec(rgb[i], rgb[j], intent, 0, px),
                                   expect_px(px, ref, 1, {ICX_MATRIX}, stats), f"bi{i}->bi{j} i{intent}"))
    # synthetic profiles: primaries x curve kinds x versions, D65 media white (absolute intent)
    kinds = ["para3", "gamma", "table", "id", "para0", "para1", "para2", "para4"]
    for n in range(40):
        prim = SRGB_PRIM if n % 2 else WIDE_PRIM
        M, w = rgb_matrix(prim, D65)
        trc = [srgb_like_curve(rng.choice(kinds), rng) for _ in range(3)]
        ver = rng.choice([0x02100000, 0x04300000])
        wt = np.array([w[0], 1.0, w[2]]) if n % 3 == 0 else None
        cls_ = mtx_profile(M, trc, ver=ver, wtpt=wt)
        other = rgb[rng.randrange(4)]
        intent = rng.choice([0, 1, 2, 3])
        flags = rng.choice([0, ICF_BPC])
        a, b = (cls_, other) if n % 2 else (other, cls_)
        ref = ref_transform(a, b, intent, flags, px)
        recs.append(it.Rec(OP_XFORM, 0, xrec(a, b, intent, flags, px),
                           expect_px(px, ref, 1, {ICX_MATRIX, ICX_IDENTITY}, stats), f"synth{n}"))
    # 0 = built-in sRGB on either side
    ref = ref_transform(bis[0], bis[1], 0, 0, px)
    recs.append(it.Rec(OP_XFORM, 0, xrec(None, bis[1], 0, 0, px), expect_px(px, ref, 1, {ICX_MATRIX}), "srgb0->adobe"))
    it.run_suite(runner, s, recs)
    s.notes.append(stat_note("vs float64 reference", stats))
    return s


def suite_gray(runner, target, rng, bis):
    s = it.Suite("gray", target)
    px = test_pixels(rng)
    recs, stats = [], []
    grays = [bis[4], gray_profile(el_curv_gamma(1.8)), gray_profile(el_para(*SRGB_PARA)),
             gray_profile(el_curv_table(np.rint(np.linspace(0, 1, 300) ** 2.0 * 65535)))]
    for g in grays:
        for other in (bis[0], bis[2]):
            for a, b in ((g, other), (other, g)):
                ref = ref_transform(a, b, 1, 0, px)
                recs.append(it.Rec(OP_XFORM, 0, xrec(a, b, 1, 0, px), expect_px(px, ref, 1, None, stats), "gray"))
        ref = ref_transform(g, bis[4], 1, 0, px)
        recs.append(it.Rec(OP_XFORM, 0, xrec(g, bis[4], 1, 0, px), expect_px(px, ref, 1, None, stats), "gray->gray"))
    it.run_suite(runner, s, recs)
    s.notes.append(stat_note("vs float64 reference", stats))
    return s


def suite_lut(runner, target, rng, bis):
    s = it.Suite("lut", target)
    px = test_pixels(rng)
    recs, st_model, st_exact, st_icf = [], [], [], []
    luts = lut_profiles()
    pairs = []
    for name, p in luts:
        if name.startswith("a2b"):
            pairs += [(name + "->srgb", p, bis[0]), (name + "->p3", p, bis[2])]
        else:
            pairs += [("srgb->" + name, bis[0], p), ("adobe->" + name, bis[1], p)]
    pairs += [("a2b_lut16_xyz->b2a_mba", luts[0][1], luts[5][1])]
    for name, a, b in pairs:
        p99 = B2A_P99 if "b2a" in name else LUT_P99
        for flags in (0, ICF_BPC):
            model = ref_transform(a, b, 0, flags, px, lut_model=True)
            exact = ref_transform(a, b, 0, flags, px)
            recs.append(it.Rec(OP_XFORM, 0, xrec(a, b, 0, flags, px), expect_px(px, model, 1, {ICX_LUT}, st_model),
                               f"{name} f{flags} model"))
            recs.append(it.Rec(OP_XFORM, 0, xrec(a, b, 0, flags, px),
                               expect_stat(px, exact, p99, LUT_MAX, {ICX_LUT}, st_exact), f"{name} f{flags} exact"))
    # ICF_LUT on matrix profiles: the grid path against its model and the exact maths
    for i, j in ((0, 1), (1, 2), (3, 0), (4, 0), (0, 4)):
        model = ref_transform(bis[i], bis[j], 1, 0, px, lut_model=True)
        exact = ref_transform(bis[i], bis[j], 1, 0, px)
        recs.append(it.Rec(OP_XFORM, 0, xrec(bis[i], bis[j], 1, ICF_LUT, px),
                           expect_px(px, model, 1, {ICX_LUT}, st_model), f"ICF_LUT bi{i}->bi{j} model"))
        recs.append(it.Rec(OP_XFORM, 0, xrec(bis[i], bis[j], 1, ICF_LUT, px),
                           expect_stat(px, exact, 1, 3, {ICX_LUT}, st_icf), f"ICF_LUT bi{i}->bi{j} exact"))
    it.run_suite(runner, s, recs)
    s.notes.append(stat_note("vs model of the 33^3 grid", st_model))
    s.notes.append(stat_note(f"LUT profiles vs exact pipeline (p99 <= {LUT_P99}, B2A {B2A_P99}; max <= {LUT_MAX})", st_exact))
    s.notes.append(stat_note("ICF_LUT (matrix profiles through the grid) vs exact (p99 <= 1, max <= 3)", st_icf))
    return s, luts


def suite_ident(runner, target, rng, bis):
    s = it.Suite("ident", target)
    px = test_pixels(rng)
    recs = []
    for i in range(5):
        recs.append(it.Rec(OP_XFORM, 0, xrec(bis[i], bis[i], 1, 0, px), expect_px(px, px, 0, {ICX_IDENTITY}), f"same bi{i}"))
    recs.append(it.Rec(OP_XFORM, 0, xrec(None, None, 0, 0, px), expect_px(px, px, 0, {ICX_IDENTITY}), "0 -> 0"))
    # a byte-different profile with the same maths as the built-in sRGB -> identity by tables
    p = Prof(bis[0])
    other = mtx_profile(p.mat, [el_para(*SRGB_PARA)] * 3, desc="another sRGB")
    other_q = profile([(b"desc", el_mluc("sRGB copy")), (b"wtpt", el_xyz(D50))] +
                      [(t, bis[0][o:o + sz]) for t, (o, sz) in p.tags.items() if t not in (b"desc", b"wtpt", b"cprt")])
    recs.append(it.Rec(OP_XFORM, 0, xrec(other_q, bis[0], 0, 0, px), expect_px(px, px, 0, {ICX_IDENTITY}), "equal maths"))
    ref = ref_transform(other, bis[0], 0, 0, px)
    recs.append(it.Rec(OP_XFORM, 0, xrec(other, bis[0], 0, 0, px), expect_px(px, ref, 1), "near sRGB"))
    recs.append(it.Rec(OP_XFORM, 0, xrec(bis[0], bis[0], 0, ICF_NOIDENT, px),
                       expect_px(px, px, 1, {ICX_MATRIX}), "NOIDENT sRGB->sRGB"))
    it.run_suite(runner, s, recs)
    return s


def suite_rows(runner, target, rng, bis):
    s = it.Suite("rows", target)
    recs = []
    luts = lut_profiles()
    for n in (1, 2, 3, 4, 5, 7, 8, 9, 31, 33):
        px = test_pixels(rng, 64)[:n * 3 + 1]
        for a, b, fl, nm in ((bis[0], bis[1], 0, "matrix"), (luts[0][1], bis[0], 0, "lut"),
                             (bis[2], bis[2], 0, "ident")):
            ref = ref_transform(a, b, 0, fl, px, lut_model=(nm == "lut")) if nm != "ident" else px
            recs.append(it.Rec(OP_XFORM, n, xrec(a, b, 0, fl, px), expect_px(px, ref, 1), f"{nm} rows of {n}"))
            recs.append(it.Rec(OP_INPLACE, n, xrec(a, b, 0, fl, px), expect_px(px, ref, 1), f"{nm} in place rows of {n}"))
    recs.append(it.Rec(OP_XFORM, 0, xrec(bis[0], bis[1], 0, 0, np.zeros((0, 4), np.uint8)),
                       lambda r, o: None if r == ICX_MATRIX and not o else f"{r} {len(o)}", "0 pixels"))
    it.run_suite(runner, s, recs)
    return s


def suite_errors(runner, target, rng, bis):
    s = it.Suite("errors", target)
    C = it.C
    px = test_pixels(rng, 4)[:8]
    M, _ = rgb_matrix(SRGB_PRIM, D65)
    trc = [el_para(*SRGB_PARA)] * 3
    good = mtx_profile(M, trc)
    cmyk = profile([(b"desc", el_mluc("cmyk")), (b"A2B0", el_lut16(np.zeros(2 ** 4 * 3), 2, nin=4, n=2))], cs=b"CMYK", pcs=b"Lab ", cls=b"prtr")
    lab = profile([(b"desc", el_mluc("lab"))], cs=b"Lab ", cls=b"spac")
    link = mtx_profile(M, trc)[:12] + b"link" + mtx_profile(M, trc)[16:]
    para5 = mtx_profile(M, [el_para(0, [2.2])[:8] + struct.pack(">HH", 5, 0) + s15(2.2) * 7] * 3)
    missing = profile([(b"desc", el_mluc("x")), (b"rXYZ", el_xyz(M[:, 0]))])
    sing = mtx_profile(np.zeros((3, 3)), trc)
    bad_lut = profile([(b"desc", el_mluc("x")), (b"A2B0", el_lut16(np.zeros(17 ** 3 * 3), 17)[:200])])
    grid1 = profile([(b"desc", el_mluc("x")), (b"A2B0", el_lut16(np.zeros(3), 1))])
    trunc = good[:100]
    v1 = bytearray(good)
    v1[8] = 1
    cases = [("cmyk", cmyk, bis[0], C.ERR_UNSUPPORTED), ("lab space", lab, bis[0], C.ERR_UNSUPPORTED),
             ("link", link, bis[0], C.ERR_UNSUPPORTED), ("para 5", para5, bis[0], C.ERR_UNSUPPORTED),
             ("missing tags", missing, bis[0], C.ERR_UNSUPPORTED), ("singular dst", bis[0], sing, C.ERR_UNSUPPORTED),
             ("truncated lut", bad_lut, bis[0], C.ERR_HDR), ("grid 1", grid1, bis[0], C.ERR_HDR),
             ("truncated", trunc, bis[0], C.ERR_TRUNC), ("version 1", bytes(v1), bis[0], C.ERR_UNSUPPORTED),
             ("dst cmyk", bis[0], cmyk, C.ERR_UNSUPPORTED)]
    recs = [it.Rec(OP_XFORM, 0, xrec(a, b, 0, 0, px), expect_err(e), n) for n, a, b, e in cases]
    recs.append(it.Rec(OP_XFORM, 0, xrec(good, bis[0], 4, 0, px), expect_err(C.ERR_UNSUPPORTED), "intent 4"))
    it.run_suite(runner, s, recs)
    return s


def suite_fuzz(runner, target, rng, bis, luts, count):
    s = it.Suite("fuzz", target)
    px = test_pixels(rng, 8)[:40]
    seeds = list(bis) + [p for _, p in luts]
    M, _ = rgb_matrix(WIDE_PRIM, D65)
    seeds.append(mtx_profile(M, [el_curv_table(np.rint(np.linspace(0, 65535, 300)))] * 3, ver=0x02100000))
    recs = []
    for k in range(count):
        a = it.mutate(rng, rng.choice(seeds), keep=rng.choice([0, 8, 128]))
        b = rng.choice(seeds)
        if k % 3 == 0:
            a, b = b, a
        op = OP_BUILD if k % 2 else OP_XFORM
        recs.append(it.Rec(op, rng.choice([0, 3]), xrec(a, b, rng.randrange(4), rng.choice([0, 1, 4]), px if op == OP_XFORM else px[:0]),
                           None, f"fuzz{k}"))
    it.run_suite(runner, s, recs)
    return s


LCMS_CODE = r"""
import ctypes, struct, sys
lc = ctypes.CDLL('liblcms2.so.2')
lc.cmsOpenProfileFromMem.restype = ctypes.c_void_p
lc.cmsOpenProfileFromMem.argtypes = [ctypes.c_char_p, ctypes.c_uint32]
lc.cmsCreateTransform.restype = ctypes.c_void_p
lc.cmsCreateTransform.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
lc.cmsDoTransform.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint32]
lc.cmsDeleteTransform.argtypes = [ctypes.c_void_p]
lc.cmsCloseProfile.argtypes = [ctypes.c_void_p]
data = sys.stdin.buffer.read()
p = 0
out = []
(n,) = struct.unpack_from('<I', data, p); p += 4
for _ in range(n):
    sl, dl, intent, flags, fin, fout, npx, bpp_in, bpp_out = struct.unpack_from('<9I', data, p); p += 36
    s = data[p:p + sl]; p += sl
    d = data[p:p + dl]; p += dl
    px = data[p:p + npx * bpp_in]; p += npx * bpp_in
    hs = lc.cmsOpenProfileFromMem(s, sl); hd = lc.cmsOpenProfileFromMem(d, dl)
    t = lc.cmsCreateTransform(hs, fin, hd, fout, intent, flags) if hs and hd else None
    if not t:
        out.append(struct.pack('<i', -1))
    else:
        buf = ctypes.create_string_buffer(npx * bpp_out)
        lc.cmsDoTransform(t, px, buf, npx)
        out.append(struct.pack('<i', npx * bpp_out) + buf.raw)
        lc.cmsDeleteTransform(t)
    if hs: lc.cmsCloseProfile(hs)
    if hd: lc.cmsCloseProfile(hd)
sys.stdout.buffer.write(b''.join(out))
"""
TYPE_BGRA_8 = 0x44499
TYPE_GRAY_8 = 0x30009
LCMS_BPC = 0x2000
LCMS_COPY_ALPHA = 0x04000000


def have_lcms():
    try:
        p = it.wsl_python("import ctypes; ctypes.CDLL('liblcms2.so.2'); print('ok')", timeout=60)
    except Exception:  # noqa: BLE001
        return False
    return p.returncode == 0 and b"ok" in p.stdout


def lcms_run(jobs):
    """jobs: (src, dst, intent, flags, px BGRA, src gray, dst gray) -> list of BGRA / None."""
    parts = [struct.pack("<I", len(jobs))]
    for src, dst, intent, flags, px, sg, dg in jobs:
        fin = TYPE_GRAY_8 if sg else TYPE_BGRA_8
        fout = TYPE_GRAY_8 if dg else TYPE_BGRA_8
        inp = px[:, 1].tobytes() if sg else px.tobytes()
        lf = (LCMS_BPC if flags & ICF_BPC else 0) | (0 if sg or dg else LCMS_COPY_ALPHA)
        parts.append(struct.pack("<9I", len(src), len(dst), intent, lf, fin, fout, len(px), 1 if sg else 4, 1 if dg else 4))
        parts += [src, dst, inp]
    p = it.wsl_python(LCMS_CODE, b"".join(parts), timeout=900)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.decode(errors="replace")[:400])
    res, q = [], 0
    for src, dst, intent, flags, px, sg, dg in jobs:
        (n,) = struct.unpack_from("<i", p.stdout, q)
        q += 4
        if n < 0:
            res.append(None)
            continue
        raw = np.frombuffer(p.stdout, np.uint8, n, q)
        q += n
        if dg:
            o = px.copy()
            o[:, 0] = o[:, 1] = o[:, 2] = raw
        else:
            o = raw.reshape(-1, 4).copy()
            if sg:
                o[:, 3] = px[:, 3]
        res.append(o)
    return res


def suite_lcms(runner, target, rng, bis, luts):
    s = it.Suite("lcms", target)
    if not have_lcms():
        s.notes.append("liblcms2 not available in WSL: skipped")
        return s
    px = test_pixels(rng)
    jobs, names, tols = [], [], []
    for i in range(4):
        for j in range(4):
            if i != j:
                for intent in (0, 1, 3):
                    jobs.append((bis[i], bis[j], intent, 0, px, False, False))
                    names.append(f"bi{i}->bi{j} i{intent}")
                    tols.append(LCMS_TOL)
    for g in (0, 2):
        jobs.append((bis[4], bis[g], 1, 0, px, True, False))
        names.append(f"gray->bi{g}")
        tols.append(LCMS_TOL)
        jobs.append((bis[g], bis[4], 1, 0, px, False, True))
        names.append(f"bi{g}->gray")
        tols.append(LCMS_GRAY_TOL)
    M, w = rgb_matrix(WIDE_PRIM, D65)
    for n in range(12):
        trc = [srgb_like_curve(rng.choice(["para3", "gamma", "table", "para0", "para4"]), rng) for _ in range(3)]
        p = mtx_profile(M, trc, ver=rng.choice([0x02100000, 0x04300000]), wtpt=np.array([w[0], 1, w[2]]))
        # (absolute intent: only against the float64 reference -- with a non-D50 media white
        # in a v4 profile lcms resamples into a clipped CLUT, up to 20 off at gamut borders)
        intent = rng.choice([0, 1])
        jobs.append((p, bis[0], intent, 0, px, False, False))
        names.append(f"synth{n} i{intent}")
        tols.append(LCMS_TOL)
    # LUT profiles with the relative colorimetric intent: for v4 profiles lcms forces black
    # point compensation with the fixed v4 perceptual black in the perceptual intent
    for name, p in luts:
        a, b = (p, bis[0]) if name.startswith("a2b") else (bis[0], p)
        jobs.append((a, b, 1, 0, px, False, False))
        names.append(name)
        tols.append(None)
    ref = lcms_run(jobs)
    recs, st_m, st_l = [], [], []
    for (src, dst, intent, flags, _px, sg, dg), nm, tol, r in zip(jobs, names, tols, ref):
        if r is None:
            s.notes.append(f"lcms refused {nm}")
            continue
        chk = (expect_stat(px, r, B2A_P99 if "b2a" in nm else LUT_P99, LUT_MAX, None, st_l) if tol is None
               else expect_px(px, r, tol, None, st_m))
        recs.append(it.Rec(OP_XFORM, 0, xrec(src, dst, intent, flags, px), chk, "lcms " + nm))
    it.run_suite(runner, s, recs)
    s.notes.append(stat_note(f"matrix/TRC vs LittleCMS (tolerance {LCMS_TOL})", st_m))
    s.notes.append(stat_note(f"LUT profiles vs LittleCMS (p99 <= {LUT_P99}, max <= {LUT_MAX})", st_l))
    return s


def suite_speed(runner, target, rng, bis, luts):
    s = it.Suite("speed", target)
    r = np.random.default_rng(7)
    n = 1 << 20
    px = r.integers(0, 256, (n, 4), dtype=np.uint8)
    reps = 8
    for nm, a, b, fl in (("matrix sRGB->Adobe", bis[0], bis[1], 0), ("3D LUT (lut16 A2B)", luts[0][1], bis[0], 0),
                         ("3D LUT forced (ICF_LUT)", bis[0], bis[2], ICF_LUT), ("identity", bis[0], bis[0], 0)):
        t0 = time.time()
        res, out = runner.one(OP_BENCH, reps, xrec(a, b, 0, fl, px))
        s.check(res > 0, f"{nm}: bench failed {res}")
        if res > 0:
            s.notes.append(f"{nm}: {n * reps / res:.0f} MPixel/s (1 thread, kind {struct.unpack('<I', out)[0]}; "
                           f"build + run {time.time() - t0:.2f} s)")
    return s


def suite_monitor(runner, target, rng):
    s = it.Suite("monitor", target)
    for arg in (-1, 0):
        res, out = runner.one(OP_MONITOR, arg & 0xFFFFFFFF, b"")
        n, = struct.unpack_from("<I", out, 0)
        key, = struct.unpack_from("<Q", out, 4)
        hdr = out[12:]
        if res == 1:
            s.check(n >= 132 and hdr[36:40] == b"acsp", f"monitor {arg}: bad profile header")
            s.notes.append(f"window {arg}: profile of {n} bytes, class {hdr[12:16]!r}, space {hdr[16:20]!r}")
        else:
            s.check(n == 0, f"monitor {arg}: length without profile")
            s.notes.append(f"window {arg}: no profile (key {key:#x})")
    return s


def main():
    ap = it.argparser(__doc__, "builtin,matrix,gray,lut,ident,rows,errors,fuzz,lcms,speed,monitor")
    ap.set_defaults(fuzz=1500)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    only = set(x for x in a.only.split(",") if x)
    if not a.no_build:
        it.build_codectest(targets, out)
    suites = []
    for t in targets:
        rng = random.Random(a.seed)
        runner = it.Runner(t, out, work_name="icc")

        def want(n):
            return not only or n in only
        sb, bis = suite_builtin(runner, t, rng)
        if want("builtin"):
            suites.append(sb)
        luts = lut_profiles()
        for name, fn in (("matrix", suite_matrix), ("gray", suite_gray), ("ident", suite_ident),
                         ("rows", suite_rows), ("errors", suite_errors)):
            if want(name):
                suites.append(fn(runner, t, rng, bis))
        if want("lut"):
            suites.append(suite_lut(runner, t, rng, bis)[0])
        if want("fuzz"):
            suites.append(suite_fuzz(runner, t, rng, bis, luts, a.fuzz))
        if want("lcms") and not a.no_ext and t == targets[0]:
            suites.append(suite_lcms(runner, t, rng, bis, luts))
        if want("speed"):
            suites.append(suite_speed(runner, t, rng, bis, luts))
        if want("monitor"):
            suites.append(suite_monitor(runner, t, rng))
    ok = it.report(suites, "icc_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
