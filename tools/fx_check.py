#!/usr/bin/env python3
"""fx_check.py -- tests of the effect / adjustment kernels (ASMLIB/src/fx) through apps/efftest.

  python tools/fx_check.py [--target win|linux|both] [--out build/fxa] [--only a,b,..]
                           [--no-build] [--quick] [--bench] [--view DIR] [--keep]

Suites (FXA): every adjustment of fx_adj.s, quantize (fx_color.s), edge_detect / emboss /
outline / relief (fx_stylize.s).  Each suite compares against a numpy reference that
implements exactly the documented formula, then runs the generic checks: tile / thread
determinism (tiles 64 / 256 / 37 / whole, --threads 1 vs the pool must be
bit-identical), every ROI pixel written (two --fill colours agree), nothing written outside
the ROI, a selection mask lerps exactly like k_lerp_mask_row, JOB_CANCEL -> ERR_CANCEL quickly,
and efftest's own guard-byte / source-unchanged checks.

Importable harness for other effect agents (FXB, FXC):

    import sys; sys.path.insert(0, "tools"); import fx_check as fc
    h = fc.Harness("win", fc.ROOT / "build/fxb")       # or "linux" (runs through WSL)
    img = fc.img_testsrc(320, 240)                      # uint8 (H, W, 4), bytes B, G, R, A
    r = h.run1("gaussian_blur", img, {"radius": 5})     # -> Result(ok, code, out, line, ...)
    fails = fc.generic_checks(h, "gaussian_blur", img, {"radius": 5})   # [] when all passed
    h.runs([...Spec...])                                # many runs in one efftest process

Images: img_gradient, img_noise, img_shapes (transparent areas with random colours),
img_allvalues (every byte value in every channel), img_testsrc / img_ffmpeg (lavfi sources:
testsrc2, mandelbrot, cellauto, smptebars, ...).  write_png(path, img) for viewing.
"""

import argparse
import math
import os
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
WSL_DISTRO = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")
RAW_MAGIC = b"RSR1"
FMT_ARGB = 3
ERR_CANCEL = -14
FXF_ADJ, FXF_PREPARE, FXF_SEED, FXF_BANDS, FXF_WHOLE, FXF_NO_DIALOG, FXF_SLOW = \
    1, 2, 4, 8, 16, 32, 64


# ---------------------------------------------------------------- raw files / png

def write_raw(path, img):
    img = np.ascontiguousarray(img, dtype=np.uint8)
    h, w = img.shape[:2]
    with open(path, "wb") as f:
        f.write(RAW_MAGIC + struct.pack("<III", w, h, FMT_ARGB))
        f.write(img.tobytes())


def read_raw(path):
    d = Path(path).read_bytes()
    if d[:4] != RAW_MAGIC:
        raise ValueError(f"{path}: not an RSR1 file")
    w, h, _ = struct.unpack_from("<III", d, 4)
    return np.frombuffer(d, np.uint8, w * h * 4, 16).reshape(h, w, 4).copy()


def write_png(path, img, bg=None):
    """RGBA PNG of a BGRA image (bg: composite over a checkerboard first when True)."""
    img = np.asarray(img, np.uint8)
    h, w = img.shape[:2]
    rgba = img[..., [2, 1, 0, 3]].copy()
    if bg:
        yy, xx = np.mgrid[0:h, 0:w]
        chk = np.where(((xx // 8) + (yy // 8)) % 2 == 0, 255, 204).astype(np.float64)
        a = rgba[..., 3:4] / 255.0
        rgba[..., :3] = np.round(rgba[..., :3] * a + chk[..., None] * (1 - a)).astype(np.uint8)
        rgba[..., 3] = 255
    raw = b"".join(b"\0" + rgba[y].tobytes() for y in range(h))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))
    Path(path).write_bytes(png)


def contact_sheet(tiles, cols=4, pad=4):
    """tiles: list of BGRA images of equal size -> one image."""
    h, w = tiles[0].shape[:2]
    rows = (len(tiles) + cols - 1) // cols
    sheet = np.full((rows * (h + pad) + pad, cols * (w + pad) + pad, 4), 255, np.uint8)
    for i, t in enumerate(tiles):
        r, c = divmod(i, cols)
        sheet[pad + r * (h + pad):pad + r * (h + pad) + h, pad + c * (w + pad):pad + c * (w + pad) + w] = t
    return sheet


# ---------------------------------------------------------------- images

def bgra(b, g, r, a):
    return np.stack([np.asarray(c, np.uint8) for c in (b, g, r, a)], -1)


def img_gradient(w=256, h=192):
    yy, xx = np.mgrid[0:h, 0:w]
    r = (xx * 255 // max(w - 1, 1))
    g = (yy * 255 // max(h - 1, 1))
    b = ((xx + yy) * 255 // max(w + h - 2, 1))
    return bgra(b, g, r, np.full((h, w), 255))


def img_noise(w=200, h=150, seed=1, alpha="mixed"):
    rng = np.random.default_rng(seed)
    c = rng.integers(0, 256, (h, w, 4), dtype=np.int64)
    if alpha == "opaque":
        c[..., 3] = 255
    else:
        sel = rng.random((h, w))
        c[..., 3] = np.where(sel < 0.4, 255, np.where(sel < 0.55, 0, c[..., 3]))
    return c.astype(np.uint8)


def img_allvalues():
    """256 x 256: B = x, G = y, R = (7x + 13y) & 255, A = (x ^ y) with 255 / 0 bands."""
    yy, xx = np.mgrid[0:256, 0:256]
    a = np.where(yy < 128, 255, (xx ^ yy) & 255)
    a = np.where((yy >= 200) & (yy < 208), 0, a)
    return bgra(xx, yy, (7 * xx + 13 * yy) & 255, a)


def img_shapes(w=240, h=180, seed=7):
    """transparent background with random colours, opaque / translucent shapes"""
    rng = np.random.default_rng(seed)
    img = rng.integers(0, 256, (h, w, 4), dtype=np.int64)
    img[..., 3] = 0
    yy, xx = np.mgrid[0:h, 0:w]
    for i in range(9):
        cx, cy, r = rng.integers(0, w), rng.integers(0, h), rng.integers(10, 50)
        col = rng.integers(0, 256, 4)
        col[3] = 255 if i % 3 else rng.integers(60, 255)
        m = (xx - cx) ** 2 + (yy - cy) ** 2 < r * r
        if i % 2:
            m = (abs(xx - cx) < r) & (abs(yy - cy) < r // 2)
        img[m] = col
    return img.astype(np.uint8)


_ffcache = {}


def img_ffmpeg(src="testsrc2", w=320, h=240):
    """a lavfi test source (testsrc2, mandelbrot, cellauto, smptebars, rgbtestsrc, ...)"""
    key = (src, w, h)
    if key not in _ffcache:
        spec = f"{src}=size={w}x{h}" if src != "cellauto" else f"cellauto=size={w}x{h}:rule=110"
        res = subprocess.run(["ffmpeg", "-loglevel", "error", "-f", "lavfi", "-i", spec,
                              "-frames:v", "1", "-pix_fmt", "bgra", "-f", "rawvideo", "-"],
                             capture_output=True)
        if res.returncode != 0 or len(res.stdout) != w * h * 4:
            raise RuntimeError(f"ffmpeg {spec}: {res.stderr.decode(errors='replace')}")
        im = np.frombuffer(res.stdout, np.uint8).reshape(h, w, 4).copy()
        im[..., 3] = 255                        # lavfi sources have no alpha (rgb0 padding)
        _ffcache[key] = im
    return _ffcache[key].copy()


def img_testsrc(w=320, h=240):
    return img_ffmpeg("testsrc2", w, h)


def img_photo(w=320, h=240):
    """smooth photographic-looking content: mandelbrot colours blurred with a box"""
    m = img_ffmpeg("mandelbrot", w, h).astype(np.float64)
    k = 5
    p = np.pad(m, ((k, k), (k, k), (0, 0)), mode="edge")
    c = np.cumsum(np.cumsum(p, 0), 1)
    c = np.pad(c, ((1, 0), (1, 0), (0, 0)))
    s = (c[2 * k + 1:, 2 * k + 1:] - c[:-2 * k - 1, 2 * k + 1:] - c[2 * k + 1:, :-2 * k - 1]
         + c[:-2 * k - 1, :-2 * k - 1]) / (2 * k + 1) ** 2
    out = np.clip(np.round(0.6 * s + 0.4 * m), 0, 255).astype(np.uint8)
    out[..., 3] = 255
    return out


def with_alpha_hole(img, seed=3):
    """copy of img with a transparent ellipse (random colours) and a translucent band"""
    out = img.copy()
    h, w = out.shape[:2]
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:h, 0:w]
    m = ((xx - w * 0.7) / (w * 0.15)) ** 2 + ((yy - h * 0.35) / (h * 0.2)) ** 2 < 1
    out[m] = rng.integers(0, 256, (int(m.sum()), 4))
    out[m, 3] = 0
    band = (yy > h * 0.8) & (yy < h * 0.9)
    out[band, 3] = 128
    return out


# ---------------------------------------------------------------- runner

class Result:
    def __init__(self, line, out=None):
        self.line = line
        t = line.split()
        self.ok = bool(t) and t[0] == "ok"
        self.fx = t[1] if len(t) > 1 else "?"
        kv = {}
        for tok in t[2:]:
            if "=" in tok:
                k, v = tok.split("=", 1)
                kv[k] = v
        self.kv = kv
        self.code = 0 if self.ok else int(kv.get("code", "-9999"))
        self.prepare_us = int(kv.get("prepare_us", "0"))
        self.render_us = int(kv.get("render_us", "0"))
        self.mpxs = int(kv.get("mpxs", "0"))
        self.out = out

    def __repr__(self):
        return f"Result({self.line!r})"


class Spec:
    """one efftest run: params = {key: value} (strings / numbers / tuples for pan)"""

    def __init__(self, fx, img=None, params=None, tile=None, threads=None, roi=None, sel=None,
                 fill=None, bench=None, cancel=False, inpath=None, keep_out=True, extra=()):
        self.fx, self.img, self.params = fx, img, dict(params or {})
        self.tile, self.threads, self.roi, self.sel, self.fill = tile, threads, roi, sel, fill
        self.bench, self.cancel, self.inpath, self.keep_out, self.extra = \
            bench, cancel, inpath, keep_out, extra


def fmt_value(v):
    if isinstance(v, bool):
        return "1" if v else "0"
    if isinstance(v, tuple):
        return ",".join(repr(float(x)) for x in v)
    if isinstance(v, float):
        return repr(v)
    if isinstance(v, (bytes, bytearray)):
        return v.hex() if v else "00"
    return str(v)


class Harness:
    def __init__(self, target, out=None, work=None, pool_threads=None):
        self.target = target
        self.out = Path(out or ROOT / "build" / "fxa").resolve()
        self.work = Path(work or self.out / "fxcheck" / target).resolve()
        self.work.mkdir(parents=True, exist_ok=True)
        self.pool_threads = pool_threads
        self.n = 0
        self._imgs = {}
        self._list = None

    def rel(self, p):
        return str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")

    def cmd(self, args):
        if self.target == "win":
            c = [str(self.out / "win" / "efftest.exe")]
        else:
            c = ["wsl.exe", "-d", WSL_DISTRO, "--cd", str(ROOT), "-e",
                 "./" + self.rel(self.out / "linux" / "efftest")]
        c += ["--shot", "-"]
        c += list(getattr(self, "proc_args", ()))   # process-level options (e.g. --simd sse2)
        if self.pool_threads:
            c += ["--threads", str(self.pool_threads)]
        return c + list(args)

    def run_cmd(self, args, timeout=3600):
        return subprocess.run(self.cmd(args), cwd=ROOT, capture_output=True, text=True,
                              timeout=timeout)

    def image_path(self, img):
        """write img once per harness (keyed by content) and return its path"""
        key = (img.shape, hash(img.tobytes()))
        p = self._imgs.get(key)
        if p is None:
            p = self.work / f"in{len(self._imgs)}.raw"
            write_raw(p, img)
            self._imgs[key] = p
        return p

    def effects(self):
        """{name: {"id", "cat", "flags", "props": {key: {...}}}} from --list"""
        if self._list is None:
            res = self.run_cmd(["--list"])
            fx = {}
            cur = None
            for line in res.stdout.splitlines():
                t = line.split()
                if not t:
                    continue
                # names may contain spaces ("prop Cyan / Rot type=int ..."): the name is
                # everything before the first key=value token
                k = next((i for i, x in enumerate(t) if "=" in x), len(t))
                name, kv = " ".join(t[1:k]), dict(x.split("=", 1) for x in t[k:])
                if t[0] == "effect":
                    cur = fx[name] = {"id": int(kv["id"]), "cat": int(kv["cat"]),
                                      "flags": int(kv["flags"]), "props": {}}
                elif t[0] == "prop" and cur is not None:
                    cur["props"][name] = kv
            self._list = fx
        return self._list

    def runs(self, specs, timeout=3600):
        """run every Spec in one efftest process -> list of Result (with .out arrays)"""
        lines, outs = [], []
        for s in specs:
            self.n += 1
            inp = s.inpath or self.image_path(s.img)
            outp = self.work / f"o{self.n}.raw"
            tok = ["--fx", s.fx, "--in", self.rel(inp)]
            if s.keep_out:
                tok += ["--out", self.rel(outp)]
            for k, v in s.params.items():
                tok += ["--p", f"{k}={fmt_value(v)}"]
            if s.tile:
                tok += ["--tile", str(s.tile)]
            if s.threads:
                tok += ["--threads", str(s.threads)]
            if s.roi:
                tok += ["--roi", ",".join(str(int(v)) for v in s.roi)]
            if s.sel is not None:
                tok += ["--sel", self.rel(self.image_path(s.sel))]
            if s.fill is not None:
                tok += ["--fill", f"{s.fill & 0xFFFFFFFF:08X}"]
            if s.bench:
                tok += ["--bench", str(s.bench)]
            if s.cancel:
                tok += ["--cancel"]
            tok += list(s.extra)
            lines.append(" ".join(tok))
            outs.append(outp if s.keep_out else None)
        opsf = self.work / f"ops{self.n}.txt"
        opsf.write_text("\n".join(lines) + "\n", encoding="utf-8")
        res = self.run_cmd(["--ops", self.rel(opsf)], timeout=timeout)
        rl = [l for l in res.stdout.splitlines() if l.startswith(("ok ", "err "))]
        results = []
        for i, s in enumerate(specs):
            if i >= len(rl):
                results.append(Result(f"err {s.fx} code=-9999 no result (process exit "
                                      f"{res.returncode}: {res.stderr.strip()[-300:]})"))
                continue
            r = Result(rl[i])
            if r.ok and outs[i] is not None and outs[i].exists():
                r.out = read_raw(outs[i])
                outs[i].unlink()
            results.append(r)
        return results

    def run1(self, fx, img, params=None, **kw):
        return self.runs([Spec(fx, img, params, **kw)])[0]


# ---------------------------------------------------------------- comparison helpers

def lerp_mask(a, b, m):
    """k_lerp_mask_row: round((a (255 - m) + b m) / 255) on all four channels"""
    a = a.astype(np.int64)
    b = b.astype(np.int64)
    m = m.astype(np.int64)[..., None]
    x = a * (255 - m) + b * m
    return (((x + 128) * 257) >> 16).astype(np.uint8)


def diff_stats(got, exp):
    d = np.abs(got.astype(np.int64) - exp.astype(np.int64))
    return int(d.max()) if d.size else 0, int((d > 0).sum())


class Report:
    def __init__(self, target):
        self.target = target
        self.fails = []
        self.passed = 0

    def check(self, cond, msg):
        if cond:
            self.passed += 1
        else:
            self.fails.append(msg)
            print(f"  FAIL [{self.target}] {msg}")
        return cond

    def compare(self, name, r, exp, tol=0, frac=None):
        """r: Result; exp: expected array; tol: max |diff|; frac: allowed fraction of
        non-zero differences (None = any within tol)"""
        if not self.check(r.ok and r.out is not None, f"{name}: {r.line}"):
            return False
        if not self.check(r.out.shape == exp.shape, f"{name}: shape {r.out.shape} != {exp.shape}"):
            return False
        mx, n = diff_stats(r.out, exp)
        ok = mx <= tol and (frac is None or n <= frac * exp.size)
        if not ok:
            d = np.abs(r.out.astype(np.int64) - exp.astype(np.int64))
            yx = np.argwhere(d.max(-1) == mx)[0]
            y, x = int(yx[0]), int(yx[1])
            msg = (f"{name}: max|d|={mx} (tol {tol}), {n} values differ; first worst at "
                   f"x={x} y={y} got {list(r.out[y, x])} exp {list(exp[y, x])}")
        else:
            msg = ""
        return self.check(ok, msg)


def generic_checks(h, fx, img, params=None, rep=None, cancel=True, sel=True, tol_sel=0,
                   slow=False, roi_independent=None):
    """determinism, fill, ROI, selection and cancel checks of one effect -> list of failures.
    The single-threaded 256-tile run is the reference; everything else must be identical."""
    rep = rep or Report(h.target)
    nf = len(rep.fails)
    info = h.effects().get(fx)
    if not rep.check(info is not None, f"{fx}: not registered"):
        return rep.fails[nf:]
    flags = info["flags"]
    H, W = img.shape[:2]
    roi = (13, 7, W - 21, H - 5)
    mask = np.zeros((H, W, 4), np.uint8)
    yy, xx = np.mgrid[0:H, 0:W]
    m = np.clip(((xx * 255) // max(W - 1, 1) + (yy % 3) * 40), 0, 255).astype(np.uint8)
    m[:, : W // 5] = 0
    m[:, -W // 6:] = 255
    mask[..., 2] = m
    mask[..., 3] = 255
    specs = [Spec(fx, img, params, tile=256, threads=1),           # 0 reference
             Spec(fx, img, params, tile=64),                        # 1 pool, 64
             Spec(fx, img, params, tile=100000),                    # 2 pool, whole
             Spec(fx, img, params, tile=37, threads=1),             # 3 odd serial
             Spec(fx, img, params, tile=48, fill=0x12345678),       # 4 fill A
             Spec(fx, img, params, tile=80, fill=0x9ABCDEF0),       # 5 fill B
             Spec(fx, img, params, tile=50, roi=roi, fill=0x5A5A5A5A)]   # 6 roi
    if sel:
        specs.append(Spec(fx, img, params, tile=64, sel=mask))      # 7 selection
    res = h.runs(specs)
    ref = res[0]
    if not rep.check(ref.ok and ref.out is not None, f"{fx} {params}: reference run: {ref.line}"):
        return rep.fails[nf:]
    names = ["pool/64", "pool/whole", "serial/37", "fill A", "fill B"]
    for i, nm in enumerate(names, 1):
        r = res[i]
        rep.check(r.ok and r.out is not None and np.array_equal(r.out, ref.out),
                  f"{fx} {params}: {nm} differs from serial/256 ({r.line[:120]}; "
                  f"{diff_stats(r.out, ref.out) if r.out is not None else ''})")
    r = res[6]
    if rep.check(r.ok and r.out is not None, f"{fx} {params}: roi run {r.line}"):
        x0, y0, x1, y1 = roi
        if flags & FXF_WHOLE:
            inside = np.ones((H, W), bool)
        elif flags & FXF_BANDS:
            inside = (yy >= y0) & (yy < y1)
        else:
            inside = (xx >= x0) & (xx < x1) & (yy >= y0) & (yy < y1)
        fillpx = np.frombuffer(struct.pack("<I", 0x5A5A5A5A), np.uint8)
        if roi_independent is None:
            roi_independent = not (flags & FXF_PREPARE)   # statistics over the ROI differ
        if roi_independent:
            rep.check(np.array_equal(r.out[inside], ref.out[inside]),
                      f"{fx} {params}: ROI content differs from the full run")
        rep.check(bool((r.out[~inside] == fillpx).all()),
                  f"{fx} {params}: pixels outside the ROI were written")
    if sel:
        r = res[7]
        # statistics effects (prepare over the selection) legitimately differ: compare only
        # where the mask is 0 (source) and check the lerp for effects without prepare
        if rep.check(r.ok and r.out is not None, f"{fx} {params}: selection run {r.line}"):
            if flags & FXF_PREPARE:
                rep.check(np.array_equal(r.out[m == 0], img[m == 0]),
                          f"{fx} {params}: selection: unselected pixels changed")
            else:
                exp = lerp_mask(img, ref.out, m)
                mx, n = diff_stats(r.out, exp)
                rep.check(mx <= tol_sel, f"{fx} {params}: selection lerp max|d|={mx} ({n})")
    if cancel:
        c = h.runs([Spec(fx, img, params, cancel=True, keep_out=False)])[0]
        rep.check(c.code == ERR_CANCEL, f"{fx} {params}: cancel -> {c.line}")
        lim = 200000 if slow else 50000
        rep.check(c.render_us < lim, f"{fx} {params}: cancelled render took {c.render_us} us")
    return rep.fails[nf:]


# ================================================================ FXA references

def luma(img):
    i = img.astype(np.int64)
    return (7471 * i[..., 0] + 38470 * i[..., 1] + 19595 * i[..., 2]) >> 16


def apply_lut(img, lut):
    """lut: (4, 256) tables B, G, R, A"""
    out = np.empty_like(img)
    for c in range(4):
        out[..., c] = np.asarray(lut[c], np.uint8)[img[..., c]]
    return out


def ident_lut():
    return np.tile(np.arange(256, dtype=np.int64), (4, 1))


def f32(x):
    return float(np.float32(x))


def level_table(in_lo, in_hi, out_lo, out_hi, gamma):
    """paint.net UnaryPixelOps.Level for one channel (fx_levels_lut)"""
    t = np.arange(256, dtype=np.int64)
    if in_hi <= in_lo:
        return t
    g = f32(gamma)
    if g != g:
        g = 1.0
    g = min(max(g, 0.1), 10.0)
    g = f32(g)
    rng = np.float32(in_hi - in_lo)
    out = []
    for j in range(256):
        if j < in_lo:
            out.append(out_lo)
        elif j >= in_hi:
            out.append(out_hi)
        else:
            x = float(np.float32(np.float32(j - in_lo) / rng))
            p = 0.0 if x <= 0 else (x if g == 1.0 else math.pow(x, g))
            v = out_lo + (out_hi - out_lo) * p
            out.append(int(min(max(v, 0.0), 255.0)))
    return np.array(out, np.int64)


def ref_invert_colors(img, p):
    out = img.copy()
    out[..., :3] = 255 - out[..., :3]
    return out


def ref_invert_alpha(img, p):
    out = img.copy()
    out[..., 3] = 255 - out[..., 3]
    return out


def ref_black_white(img, p):
    out = img.copy()
    i = luma(img).astype(np.uint8)
    out[..., 0] = out[..., 1] = out[..., 2] = i
    return out


def sepia_tables():
    return (level_table(0, 255, 0, 255, 1.2), np.arange(256), level_table(0, 255, 0, 255, 0.8))


def ref_sepia(img, p):
    tb, tg, tr = sepia_tables()
    i = luma(img)
    out = img.copy()
    out[..., 0] = tb[i]
    out[..., 1] = tg[i]
    out[..., 2] = tr[i]
    return out


def tdiv(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def clampi(v, lo, hi):
    return max(lo, min(hi, int(v)))


def ref_brightness_contrast(img, p):
    b = clampi(p.get("brightness", 0), -100, 100)
    c = clampi(p.get("contrast", 0), -100, 100)
    if c < 0:
        mul, div = c + 100, 100
    elif c > 0:
        mul, div = 100, 100 - c
    else:
        mul, div = 1, 1
    i = luma(img)
    out = img.copy()
    if div == 0:
        tab = np.array([0 if k + b < 128 else 255 for k in range(256)])
        v = tab[i].astype(np.uint8)
        out[..., 0] = out[..., 1] = out[..., 2] = v
        return out
    if div == 100:
        shift = np.array([tdiv((k - 127) * mul, div) + 127 - k + b for k in range(256)])
    else:
        shift = np.array([tdiv((k - 127 + b) * mul, div) + 127 - k for k in range(256)])
    s = shift[i]
    for ch in range(3):
        out[..., ch] = np.clip(img[..., ch].astype(np.int64) + s, 0, 255)
    return out


def posterize_levels(L):
    t1 = [0] + [(255 * i) // (L - 1) for i in range(1, L)]
    out, j, k = [], 0, 0
    for i in range(256):
        out.append(t1[j])
        k += L
        if k > 255:
            k -= 255
            j += 1
    return np.array(out)


def ref_posterize(img, p):
    lv = [clampi(p.get(k, 16), 2, 64) for k in ("red", "green", "blue")]
    if int(p.get("linked", 1)):
        lv = [lv[0]] * 3
    lut = ident_lut()
    lut[2], lut[1], lut[0] = posterize_levels(lv[0]), posterize_levels(lv[1]), posterize_levels(lv[2])
    return apply_lut(img, lut)


def ref_hue_saturation(img, p):
    hue = clampi(p.get("hue", 0), -180, 180)
    sat = clampi(p.get("saturation", 100), 0, 200)
    light = clampi(p.get("lightness", 0), -100, 100)
    sf = (sat * 1024) // 100
    if hue == 0 and sf == 1024 and light == 0:
        return img.copy()
    c = img[..., :3].astype(np.int64)            # B, G, R
    if sf != 1024:
        i = luma(img)[..., None]
        v = i * 1024 + (c - i) * sf
        c = np.minimum(255, np.maximum(v, 0) >> 10)
    if hue != 0:
        B, G, R = c[..., 0], c[..., 1], c[..., 2]
        mx = np.maximum(np.maximum(B, G), R)
        mn = np.minimum(np.minimum(B, G), R)
        d = mx - mn
        mR = R == mx
        mG = (G == mx) & ~mR
        num = np.where(mR, G - B, np.where(mG, B - R, R - G))
        base = np.where(mR, 0, np.where(mG, 2, 4))
        D = 60 * (d * base + num) + hue * d
        P = 360 * d
        outc = []
        for n in (1, 3, 5):                       # B, G, R
            K = D + 60 * n * d
            K = np.where(K < 0, K + P, K)
            K = np.where(K >= P, K - P, K)
            K = np.where(K >= P, K - P, K)
            X = np.clip(np.minimum(K, 240 * d - K), 0, 60 * d)
            outc.append(mx - (X + 29) // 60)
        c = np.stack(outc, -1)
    if light != 0:
        a = (abs(light) * 255) // 100
        col = 255 if light > 0 else 0
        c = (c * (255 - a) + col * a) >> 8
    out = img.copy()
    out[..., :3] = c.astype(np.uint8)
    return out


def color_bytes(v, default):
    if isinstance(v, str):
        s = v.lstrip("#")
        if s.lower().startswith("0x"):
            s = s[2:]
        x = int(s, 16)
        if len(s) <= 6:
            x |= 0xFF000000
    elif v is None:
        x = default
    else:
        x = int(v)
    return [x & 255, (x >> 8) & 255, (x >> 16) & 255, (x >> 24) & 255]


def ref_levels(img, p):
    ilo = color_bytes(p.get("input_black"), 0xFF000000)
    ihi = color_bytes(p.get("input_white"), 0xFFFFFFFF)
    olo = color_bytes(p.get("output_black"), 0xFF000000)
    ohi = color_bytes(p.get("output_white"), 0xFFFFFFFF)
    gam = [float(p.get("gamma_blue", 1.0)), float(p.get("gamma_green", 1.0)),
           float(p.get("gamma_red", 1.0))]
    mask = [int(p.get("blue", 1)), int(p.get("green", 1)), int(p.get("red", 1))]
    lut = ident_lut()
    for c in range(3):
        if mask[c]:
            lut[c] = level_table(ilo[c], ihi[c], olo[c], ohi[c], gam[c])
    return apply_lut(img, lut)


def ref_auto_level(img, p, sel=None):
    ok = img[..., 3] > 0
    if sel is not None:
        ok &= sel > 0
    total = int(ok.sum())
    lut = ident_lut()
    lo, hi, md, gam = [], [], [], []
    for c in range(3):
        hist = np.bincount(img[..., c][ok], minlength=256)
        cum = np.cumsum(hist)

        def perc(fr):
            t = total * float(np.float32(fr))
            idx = np.nonzero(cum > t)[0]
            return int(idx[0]) if idx.size else 0
        l, hh = perc(0.005), perc(0.995)
        s = int((np.arange(256) * hist).sum())
        mean = np.float32(s) / np.float32(total) if total else np.float32(0)
        m = int(np.float32(mean) + np.float32(0.5)) & 255
        if l < m < hh:
            r = np.float32(np.float32(m - l) / np.float32(hh - l))
            g = f32(min(max(math.log(0.5) / math.log(float(r)), 0.1), 10.0))
        else:
            g = 1.0
        lut[c] = level_table(l, hh, 0, 255, g)
    return apply_lut(img, lut)


def spline_table(pts):
    """natural cubic spline through the points (lut.rs spline_lut, the values of paint.net's
    Curves) -> ClampToByte table"""
    if not pts:
        return np.arange(256)
    d = {}
    for x, y in pts:
        d[float(x)] = float(y)
    xs = sorted(d)
    ys = [d[x] for x in xs]
    n = len(xs)
    if n == 1:
        return np.full(256, int(ys[0]))
    # second derivatives m: mu_i m[i-1] + 2 m[i] + (1 - mu_i) m[i+1] = 6 (s_i - s_(i-1)) / span,
    # m = 0 at both ends; Thomas algorithm (cp, rp: forward elimination, then back substitution)
    cp, rp, m = [0.0] * n, [0.0] * n, [0.0] * n
    for i in range(1, n - 1):
        span = xs[i + 1] - xs[i - 1]
        mu = (xs[i] - xs[i - 1]) / span
        pivot = 2.0 - mu * cp[i - 1]
        cp[i] = (1.0 - mu) / pivot
        bend = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]) - (ys[i] - ys[i - 1]) / (xs[i] - xs[i - 1])
        rp[i] = (bend * 6.0 / span - mu * rp[i - 1]) / pivot
    for i in reversed(range(n - 1)):
        m[i] = rp[i] - cp[i] * m[i + 1]
    out, seg = [], 0
    for xi in range(256):
        x = float(xi)
        while seg + 2 < n and xs[seg + 1] <= x:
            seg += 1
        x0, x1 = xs[seg], xs[seg + 1]
        h = x1 - x0
        w0, w1 = (x1 - x) / h, (x - x0) / h
        v = w0 * ys[seg] + w1 * ys[seg + 1] + ((w0 * w0 * w0 - w0) * m[seg] + (w1 * w1 * w1 - w1) * m[seg + 1]) * (h * h) / 6.0
        out.append(255 if v > 255 else (0 if not v > 0 else int(v)))
    return np.array(out)


def curve_blob(pts):
    b = bytes([len(pts)]) + b"".join(bytes([x, y]) for x, y in pts)
    return b


def ref_curves(img, p):
    mode = p.get("mode", 0)
    mode = 1 if mode in (1, "1", "rgb") else 0

    def pts(key):
        v = p.get(key)
        if not v:
            return []
        b = bytes.fromhex(v) if isinstance(v, str) else bytes(v)
        n = min(b[0], 19)
        return [(b[1 + 2 * i], b[2 + 2 * i]) for i in range(n)]
    if mode == 0:
        cur = spline_table(pts("luminosity"))
        i = luma(img)
        diff = cur[i] - i
        out = img.copy()
        for c in range(3):
            out[..., c] = np.clip(img[..., c].astype(np.int64) + diff, 0, 255)
        return out
    lut = ident_lut()
    lut[2], lut[1], lut[0] = spline_table(pts("red")), spline_table(pts("green")), spline_table(pts("blue"))
    return apply_lut(img, lut)


def srgb_dec(c):
    return c / 12.92 if c <= 0.04045 else math.pow((c + 0.055) / 1.055, 2.4)


def srgb_enc(v):
    v = min(max(v, 0.0), 1.0)
    if v <= 0.0031308:
        return v * 12.92
    return min(1.055 * math.pow(v, 1 / 2.4) - 0.055, 1.0)


def gain_table(g):
    return np.array([int(srgb_enc(srgb_dec(j / 255.0) * g) * 255.0 + 0.5) for j in range(256)])


def ref_exposure(img, p):
    ev = float(p.get("exposure", 0.0))
    ev = min(max(ev, -5.0), 5.0)
    g = 2.0 ** ev
    lut = ident_lut()
    lut[0] = lut[1] = lut[2] = gain_table(g)
    return apply_lut(img, lut)


def ref_temperature_tint(img, p):
    t = clampi(p.get("temperature", 0), -100, 100) / 100.0
    m = clampi(p.get("tint", 0), -100, 100) / 100.0
    gb = 2.0 ** (t * -0.5 + m * 0.15)
    gg = 2.0 ** (m * -0.3)
    gr = 2.0 ** (t * 0.5 + m * 0.15)
    y = gr * 0.2126 + gg * 0.7152 + gb * 0.0722
    lut = ident_lut()
    lut[0], lut[1], lut[2] = gain_table(gb / y), gain_table(gg / y), gain_table(gr / y)
    return apply_lut(img, lut)


def ref_highlights_shadows(img, p):
    """float32 mirror of fx_hx_prepare / hx_row"""
    S = clampi(p.get("shadows", 0), -100, 100)
    Hl = clampi(p.get("highlights", 0), -100, 100)
    rad = clampi(p.get("radius", 30), 1, 200)
    H, W = img.shape[:2]
    s = max(1, rad >> 2)
    GW, GH = (W + s - 1) // s, (H + s - 1) // s
    a = img[..., 3].astype(np.int64)
    ai = luma(img) * a
    pw, ph = GW * s - W, GH * s - H
    ai = np.pad(ai, ((0, ph), (0, pw)))
    ap = np.pad(a, ((0, ph), (0, pw)))
    sy = ai.reshape(GH, s, GW, s).sum((1, 3))
    sa = ap.reshape(GH, s, GW, s).sum((1, 3))
    grid = np.where(sa > 0, np.float32(sy) / np.float32(np.where(sa > 0, sa, 1) * 255),
                    np.float32(0.5)).astype(np.float32)
    sigma = rad / (2.0 * s)
    R = int(math.ceil(sigma * 3.0))
    R = min(R, 40)
    ws = [2.0 ** (-(i * i) / (sigma * sigma * 2.0 * 0.6931471805599453)) for i in range(-R, R + 1)]
    tot = sum(ws)
    wf = [np.float32(float(np.float32(w)) / tot) for w in ws]

    def blur(g, axis):
        acc = np.zeros_like(g, np.float32)
        n = g.shape[axis]
        idx = np.arange(n)
        for k, i in enumerate(range(-R, R + 1)):
            j = np.clip(idx + i, 0, n - 1)
            sh = g[:, j] if axis == 1 else g[j, :]
            acc = (acc + (sh * wf[k]).astype(np.float32)).astype(np.float32)
        return acc
    grid = blur(blur(grid, 1), 0)
    lin = np.array([np.float32(srgb_dec(j / 255.0)) for j in range(256)], np.float32)
    enc = np.array([int(srgb_enc(i / 4096.0) * 255.0 + 0.5) for i in range(4097)])
    gains = []
    for i in range(1025):
        mm = i / 1024.0
        e = (1.0 - mm) * (1.0 - mm) * (S / 50.0) - mm * mm * (Hl / 50.0)
        gains.append(np.float32(2.0 ** e))
    gains = np.array(gains, np.float32)
    invs = np.float32(1.0) / np.float32(s)
    half = np.float32(0.5)

    def coord(v, n):
        g = (np.float32(v) + half) * invs - half
        g = np.minimum(np.maximum(g, np.float32(0)), np.float32(n - 1)).astype(np.float32)
        i0 = g.astype(np.int64)
        f = (g - i0.astype(np.float32)).astype(np.float32)
        i1 = np.minimum(i0 + 1, n - 1)
        return i0, i1, f
    y0, y1, fy = coord(np.arange(H, dtype=np.float32), GH)
    x0, x1, fx = coord(np.arange(W, dtype=np.float32), GW)
    fyb = fy[:, None]
    v0 = (grid[y0] + ((grid[y1] - grid[y0]) * fyb).astype(np.float32)).astype(np.float32)
    a0 = v0[:, x0]
    a1 = v0[:, x1]
    mval = (a0 + ((a1 - a0) * fx[None, :]).astype(np.float32)).astype(np.float32)
    gi = np.clip((mval * np.float32(1024) + half).astype(np.int64), 0, 1024)
    gain = gains[gi]
    out = img.copy()
    for c in range(3):
        v = np.minimum((lin[img[..., c]] * gain).astype(np.float32), np.float32(1.0))
        idx = (v * np.float32(4096) + half).astype(np.int64)
        out[..., c] = enc[idx]
    return out


# ---------------------------------------------------------------- FXA suites

def adj_cases(quick):
    """(effect, params, reference, tolerance, fraction of differing values allowed)"""
    C = []
    C += [("invert_colors", {}, ref_invert_colors, 0, None),
          ("invert_alpha", {}, ref_invert_alpha, 0, None),
          ("black_white", {}, ref_black_white, 0, None),
          ("sepia", {}, ref_sepia, 0, None)]
    for b, c in [(0, 0), (20, 40), (-50, 0), (0, -60), (100, 100), (-100, -100), (35, 99),
                 (0, 100), (-30, 50), (500, -500)]:
        C.append(("brightness_contrast", {"brightness": b, "contrast": c},
                  ref_brightness_contrast, 0, None))
    for p in [{}, {"red": 2}, {"red": 64}, {"red": 5, "green": 9, "blue": 3, "linked": 0},
              {"red": 1, "linked": 1}, {"red": 100, "green": 2, "blue": 30, "linked": 0}]:
        C.append(("posterize", p, ref_posterize, 0, None))
    for p in [{}, {"hue": 90}, {"hue": -180}, {"hue": 37, "saturation": 150},
              {"saturation": 0}, {"saturation": 200}, {"lightness": 50}, {"lightness": -100},
              {"hue": 120, "saturation": 60, "lightness": 30}, {"hue": 400, "lightness": 300}]:
        C.append(("hue_saturation", p, ref_hue_saturation, 0, None))
    for p in [{}, {"gamma_red": 2.0, "gamma_blue": 0.5},
              {"input_black": "203040", "input_white": "E0D0C0", "output_black": "101010",
               "output_white": "F0F0F0", "gamma_green": 1.7},
              {"input_black": "404040", "input_white": "404040"},
              {"output_black": "FFFFFF", "output_white": "000000"},
              {"gamma_red": 0.01, "gamma_green": 50.0, "red": 0},
              {"input_white": "808080", "blue": 0, "green": 0}]:
        C.append(("levels", p, ref_levels, 0, None))
    C.append(("auto_level", {}, ref_auto_level, 1, 0.001))
    for p in [{}, {"luminosity": curve_blob([(0, 0), (64, 100), (255, 255)])},
              {"luminosity": curve_blob([(0, 255), (255, 0)])},
              {"mode": "rgb", "red": curve_blob([(0, 30), (128, 200), (200, 180), (255, 255)]),
               "green": curve_blob([(10, 0), (250, 255)]),
               "blue": curve_blob([(128, 64)])},
              {"mode": 1, "red": curve_blob([(255, 0), (0, 255), (128, 0), (60, 250), (190, 10)])},
              {"luminosity": curve_blob([(30, 0), (60, 40), (90, 200), (120, 255), (200, 128),
                                         (220, 230), (240, 10), (250, 250)])}]:
        C.append(("curves", p, ref_curves, 0, None))
    for ev in [0.0, 1.0, -1.5, 2.37, 5.0, -9.0]:
        C.append(("exposure", {"exposure": ev}, ref_exposure, 1, 0.001))
    for t, m in [(0, 0), (60, 0), (-100, 30), (40, -80), (100, 100)]:
        C.append(("temperature_tint", {"temperature": t, "tint": m}, ref_temperature_tint, 1, 0.001))
    for p in [{}, {"shadows": 80}, {"highlights": 70}, {"shadows": -50, "highlights": -60, "radius": 7},
              {"shadows": 100, "highlights": 100, "radius": 200}, {"shadows": 60, "radius": 2}]:
        C.append(("highlights_shadows", p, ref_highlights_shadows, 1, 0.01))
    if quick:
        seen, Q = set(), []
        for c in C:
            if c[0] not in seen:
                seen.add(c[0])
                Q.append(c)
        return Q
    return C


def suite_adjust(h, rep, only, quick):
    imgs = [("allvalues", img_allvalues()), ("shapes", img_shapes()),
            ("noise", img_noise(173, 91))]
    cases = [c for c in adj_cases(quick) if not only or c[0] in only]
    specs, exp = [], []
    for fx, p, ref, tol, frac in cases:
        for iname, img in imgs:
            specs.append(Spec(fx, img, p, tile=64))
            exp.append((f"{fx} {p} on {iname}", ref(img, p), tol, frac))
    for r, (name, e, tol, frac) in zip(h.runs(specs), exp):
        rep.compare(name, r, e, tol, frac)
    # auto level through a selection: statistics over the selected pixels only
    if not only or "auto_level" in only:
        img = img_photo(160, 120)
        sel = np.zeros(img.shape, np.uint8)
        sel[30:90, 40:120, 2] = 255
        sel[..., 3] = 255
        r = h.runs([Spec("auto_level", img, {}, sel=sel)])[0]
        e = lerp_mask(img, ref_auto_level(img, {}, sel[..., 2]), sel[..., 2])
        rep.compare("auto_level with selection", r, e, 1, 0.001)
    # generic checks (determinism, fill, roi, selection, cancel)
    gimg = with_alpha_hole(img_testsrc(300, 200))
    done = set()
    for fx, p, ref, tol, frac in cases:
        if fx in done and quick:
            continue
        if fx in done and p == {}:
            continue
        if fx in done and fx not in ("hue_saturation", "curves"):
            continue
        done.add(fx)
        generic_checks(h, fx, gimg, p, rep,
                       roi_independent=fx in ("highlights_shadows", "curves", "levels",
                                              "exposure", "temperature_tint") or None)
    # defaults and property lists
    info = h.effects()
    expect = {
        "auto_level": (FXF_ADJ | FXF_PREPARE | FXF_NO_DIALOG, {}),
        "black_white": (FXF_ADJ | FXF_NO_DIALOG, {}),
        "brightness_contrast": (FXF_ADJ, {"brightness": ("int", -100, 100, 0),
                                          "contrast": ("int", -100, 100, 0)}),
        "hue_saturation": (FXF_ADJ, {"hue": ("int", -180, 180, 0),
                                     "saturation": ("int", 0, 200, 100),
                                     "lightness": ("int", -100, 100, 0)}),
        "posterize": (FXF_ADJ, {"red": ("int", 2, 64, 16), "linked": ("bool", 0, 1, 1)}),
        "invert_colors": (FXF_ADJ | FXF_NO_DIALOG, {}),
        "invert_alpha": (FXF_ADJ | FXF_NO_DIALOG, {}),
        "sepia": (FXF_ADJ | FXF_NO_DIALOG, {}),
        "levels": (FXF_ADJ | FXF_PREPARE, {"gamma_red": ("dec", 0.1, 10, 1)}),
        "curves": (FXF_ADJ | FXF_PREPARE, {"mode": ("choice", 0, 1, 0)}),
    }
    for fx, (fl, props) in expect.items():
        if only and fx not in only:
            continue
        e = info.get(fx)
        if not rep.check(e is not None, f"{fx} registered"):
            continue
        rep.check(e["flags"] == fl, f"{fx}: flags {e['flags']} != {fl}")
        for k, (t, lo, hi, d) in props.items():
            pr = e["props"].get(k)
            if rep.check(pr is not None, f"{fx}.{k} exists"):
                rep.check(pr["type"] == t and float(pr["min"]) == lo and float(pr["max"]) == hi
                          and float(pr["def"]) == d, f"{fx}.{k}: {pr}")


# ---------------------------------------------------------------- stylize references

def stylize_weights(angle, center):
    r = angle * 2.0 * math.pi / 360.0
    dr = math.pi / 4.0
    w = [[math.cos(r + dr), math.cos(r + 2 * dr), math.cos(r + 3 * dr)],
         [math.cos(r), center, math.cos(r + 4 * dr)],
         [math.cos(r - dr), math.cos(r - 2 * dr), math.cos(r - 3 * dr)]]
    return [[int(math.floor(v * 16384 + 0.5)) for v in row] for row in w]


def conv3(img, wq, chans):
    """sum over the 3x3 taps inside the image of w * c (integer), per channel"""
    H, W = img.shape[:2]
    p = np.pad(img.astype(np.int64), ((1, 1), (1, 1), (0, 0)))
    inside = np.pad(np.ones((H, W), np.int64), 1)
    acc = np.zeros((H, W, len(chans)), np.int64)
    for fy in range(3):
        for fx in range(3):
            ww = wq[fy][fx]
            if ww == 0:
                continue
            m = inside[fy:fy + H, fx:fx + W][..., None]
            acc += ww * p[fy:fy + H, fx:fx + W][..., chans] * m
    return acc


def trunc_div(a, s):
    return np.where(a >= 0, a >> s, -((-a) >> s))


def angle_of(p, default):
    a = float(p.get("angle", default))
    return a


def ref_edge_detect(img, p):
    wq = stylize_weights(angle_of(p, 45.0), 0.0)
    acc = conv3(img, wq, [0, 1, 2])
    v = np.clip(trunc_div(acc, 14) + 128, 0, 255)
    out = img.copy()
    out[..., :3] = v
    return out


def ref_relief(img, p):
    wq = stylize_weights(angle_of(p, 45.0), 1.0)
    acc = conv3(img, wq, [0, 1, 2])
    v = np.clip(trunc_div(acc, 14), 0, 255)
    out = img.copy()
    out[..., :3] = v
    return out


def ref_emboss(img, p):
    wq = stylize_weights(angle_of(p, 0.0), 0.0)
    i = luma(img)[..., None]
    acc = conv3(np.concatenate([i, i, i, i], -1), wq, [0])[..., 0]
    v = np.clip(trunc_div(acc, 14) + 128, 0, 255).astype(np.uint8)
    out = img.copy()
    out[..., 0] = out[..., 1] = out[..., 2] = v
    return out


def ref_outline(img, p):
    rad = clampi(p.get("thickness", 3), 1, 200)
    inten = clampi(p.get("intensity", 50), 0, 100)
    H, W = img.shape[:2]
    cutoff = ((rad * 2 + 1) ** 2 + 2) // 4
    offs = [(u, v) for v in range(-rad, rad + 1) for u in range(-rad, rad + 1)
            if u * u + v * v <= cutoff]
    # per pixel histograms via one-hot accumulation (small images only)
    hist = np.zeros((H, W, 4, 256), np.int32)
    area = np.zeros((H, W), np.int64)
    yy, xx = np.mgrid[0:H, 0:W]
    for u, v in offs:
        sy, sx = yy + v, xx + u
        ok = (sy >= 0) & (sy < H) & (sx >= 0) & (sx < W)
        area += ok
        px = img[np.clip(sy, 0, H - 1), np.clip(sx, 0, W - 1)]
        for c in range(4):
            np.add.at(hist, (yy[ok], xx[ok], c, px[..., c][ok]), 1)
    out = np.empty_like(img)
    for y in range(H):
        for x in range(W):
            a = int(area[y, x])
            mc1 = a * (100 - inten) // 200
            mc2 = a * (100 + inten) // 200
            res = []
            hb = hist[y, x, 0]
            for c in range(4):
                hc = hist[y, x, c]
                h0 = hb if c == 3 else hc      # paint.net's alpha loop skips on the blue histogram
                cnt = 0
                c1 = 0
                while c1 < 255 and h0[c1] == 0:
                    c1 += 1
                while c1 < 255 and cnt < mc1:
                    cnt += int(hc[c1])
                    c1 += 1
                c2 = c1
                while c2 < 255 and cnt < mc2:
                    cnt += int(hc[c2])
                    c2 += 1
                res.append((c1, c2))
            out[y, x, 0] = 255 - (res[0][1] - res[0][0])
            out[y, x, 1] = 255 - (res[1][1] - res[1][0])
            out[y, x, 2] = 255 - (res[2][1] - res[2][0])
            out[y, x, 3] = res[3][1]
    return out


def suite_stylize(h, rep, only, quick):
    names = ["edge_detect", "emboss", "relief", "outline"]
    if only and not any(n in only for n in names):
        return
    info = h.effects()
    img = with_alpha_hole(img_testsrc(120, 90))
    small = img_noise(41, 33, seed=5)
    cell = img_ffmpeg("cellauto", 64, 40)
    cases = []
    for a in [0.0, 45.0, 90.0, 137.5, -30.0, 180.0, 359.0]:
        cases += [("edge_detect", {"angle": a}, ref_edge_detect, 0),
                  ("emboss", {"angle": a}, ref_emboss, 0),
                  ("relief", {"angle": a}, ref_relief, 0)]
    cases += [("edge_detect", {}, ref_edge_detect, 0), ("emboss", {}, ref_emboss, 0),
              ("relief", {}, ref_relief, 0)]
    ocases = [{}, {"thickness": 1, "intensity": 0}, {"thickness": 5, "intensity": 100},
              {"thickness": 2, "intensity": 73}]
    cases += [("outline", p, ref_outline, 0) for p in ocases]
    if quick:
        cases = cases[:3] + cases[-4:-3]
    cases = [c for c in cases if not only or c[0] in only]
    cases = [c for c in cases if c[0] in info]
    specs, exp = [], []
    for fx, p, ref, tol in cases:
        for iname, im in (("testsrc", img), ("noise", small), ("cellauto", cell)):
            if fx == "outline" and iname != "noise" and p.get("thickness", 3) > 3:
                continue
            specs.append(Spec(fx, im, p, tile=32))
            exp.append((f"{fx} {p} on {iname}", ref(im, p), tol))
    for r, (name, e, tol) in zip(h.runs(specs), exp):
        rep.compare(name, r, e, tol)
    gimg = with_alpha_hole(img_testsrc(200, 150))
    for fx in names:
        if fx in info and (not only or fx in only):
            generic_checks(h, fx, gimg, {"angle": 30.0} if fx != "outline" else {"thickness": 4},
                           rep, slow=fx == "outline")
    # outline: large radius and ROI border behaviour against the sliding window
    if "outline" in info and (not only or "outline" in only) and not quick:
        big = img_photo(96, 72)
        r = h.runs([Spec("outline", big, {"thickness": 20, "intensity": 30}, tile=40)])[0]
        rep.compare("outline thickness 20", r, ref_outline(big, {"thickness": 20, "intensity": 30}), 0)


# ---------------------------------------------------------------- quantize reference / suite

# ---------------------------------------------------------------- quantize reference

def _q_choice(v, names):
    if isinstance(v, str) and not v.isdigit():
        return names.index(v)
    return 1 if int(v) else 0


def q_considered(img, alpha_bin, sel=None):
    a = img[..., 3].astype(np.int64)
    ok = a >= 128 if alpha_bin else a > 0
    if sel is not None:
        ok &= sel > 0
    return ok


def q_octree(cols, K):
    """Gervautz-Purgathofer octree (see fx_color.s) -> palette list of (r, g, b)"""
    nodes = [None, {"ch": [0] * 8, "leaf": False, "n": 0, "s": [0, 0, 0], "next": 0}]
    red = [0] * 9
    red[0] = 1
    free = []
    leaves = 0

    def new():
        nd = {"ch": [0] * 8, "leaf": False, "n": 0, "s": [0, 0, 0], "next": 0}
        if free:
            i = free.pop()
            nodes[i] = nd
        else:
            nodes.append(nd)
            i = len(nodes) - 1
        return i
    for (r, g, b) in cols:
        n, L = 1, 0
        while not nodes[n]["leaf"]:
            sh = 7 - L
            ci = (((r >> sh) & 1) << 2) | (((g >> sh) & 1) << 1) | ((b >> sh) & 1)
            c = nodes[n]["ch"][ci]
            if c == 0:
                c = new()
                if L + 1 == 8:
                    nodes[c]["leaf"] = True
                    leaves += 1
                else:
                    nodes[c]["next"] = red[L + 1]
                    red[L + 1] = c
                nodes[n]["ch"][ci] = c
            n, L = c, L + 1
        nd = nodes[n]
        nd["n"] += 1
        nd["s"][0] += r
        nd["s"][1] += g
        nd["s"][2] += b
        while leaves > K:
            lv = 7
            while lv > 0 and red[lv] == 0:
                lv -= 1
            m = red[lv]
            red[lv] = nodes[m]["next"]
            cnt = 0
            for ci in range(8):
                c = nodes[m]["ch"][ci]
                if c:
                    nodes[m]["n"] += nodes[c]["n"]
                    for k in range(3):
                        nodes[m]["s"][k] += nodes[c]["s"][k]
                    free.append(c)          # LIFO free list like the assembly
                    nodes[m]["ch"][ci] = 0
                    cnt += 1
            nodes[m]["leaf"] = True
            leaves -= cnt - 1
    pal = []

    def walk(n):
        nd = nodes[n]
        if nd["leaf"]:
            if nd["n"]:
                pal.append(tuple((sv + nd["n"] // 2) // nd["n"] for sv in nd["s"]))
            return
        for c in nd["ch"]:
            if c:
                walk(c)
    walk(1)
    return pal


def q_mcut(cols, K):
    H = np.zeros((32, 32, 32), np.int64)
    S = np.zeros((32, 32, 32, 3), np.int64)
    if cols:
        a = np.array(cols, np.int64)
        k = (a[:, 0] >> 3, a[:, 1] >> 3, a[:, 2] >> 3)
        np.add.at(H, k, 1)
        for c in range(3):
            np.add.at(S[..., c], k, a[:, c])

    def shrink(lo, hi):
        sub = H[lo[0]:hi[0] + 1, lo[1]:hi[1] + 1, lo[2]:hi[2] + 1]
        cnt = int(sub.sum())
        if cnt == 0:
            return lo, hi, 0
        nz = np.nonzero(sub)
        nlo = [lo[a] + int(nz[a].min()) for a in range(3)]
        nhi = [lo[a] + int(nz[a].max()) for a in range(3)]
        return nlo, nhi, cnt
    boxes = []
    lo, hi, cnt = shrink([0, 0, 0], [31, 31, 31])
    if cnt:
        boxes.append([lo, hi, cnt])
    while boxes and len(boxes) < K:
        pick, best = -1, 0
        for i, (lo, hi, cnt) in enumerate(boxes):
            if lo != hi and cnt > best:
                pick, best = i, cnt
        if pick < 0:
            break
        lo, hi, cnt = boxes[pick]
        ext = [hi[a] - lo[a] for a in range(3)]
        ax = 0
        if ext[1] > ext[ax]:
            ax = 1
        if ext[2] > ext[ax]:
            ax = 2
        sub = H[lo[0]:hi[0] + 1, lo[1]:hi[1] + 1, lo[2]:hi[2] + 1]
        sl = sub.sum(axis=tuple(a for a in range(3) if a != ax))
        split = hi[ax] - 1
        cum = 0
        for s_ in range(lo[ax], hi[ax]):
            cum += int(sl[s_ - lo[ax]])
            if 2 * cum >= cnt:
                split = s_
                break
        hiA = list(hi)
        hiA[ax] = split
        loB = list(lo)
        loB[ax] = split + 1
        a = shrink(list(lo), hiA)
        b = shrink(loB, list(hi))
        boxes[pick] = list(a)
        boxes.append(list(b))
    pal = []
    for lo, hi, cnt in boxes:
        sub = S[lo[0]:hi[0] + 1, lo[1]:hi[1] + 1, lo[2]:hi[2] + 1].reshape(-1, 3).sum(0)
        c = int(H[lo[0]:hi[0] + 1, lo[1]:hi[1] + 1, lo[2]:hi[2] + 1].sum())
        pal.append(tuple(int((int(v) + c // 2) // c) for v in sub))
    return pal


def ref_quantize(img, p, sel=None):
    K = clampi(p.get("colors", 16), 2, 256)
    alg = _q_choice(p.get("algorithm", 0), ["octree", "median_cut"])
    L = clampi(p.get("dithering", 7), 0, 8)
    abin = _q_choice(p.get("alpha", 0), ["keep", "binary"])
    ok = q_considered(img, abin, sel)
    H, W = img.shape[:2]
    cols = [(int(c[2]), int(c[1]), int(c[0])) for c in img[ok]]
    pal = []
    seen = set()
    exact = True
    for c in cols:
        if c not in seen:
            if len(pal) == K:
                exact = False
                break
            seen.add(c)
            pal.append(c)
    if not exact:
        pal = q_octree(cols, K) if alg == 0 else q_mcut(cols, K)
    if not pal:
        return img.copy()
    P = np.array(pal, np.int64)                  # (n, 3) r, g, b
    out = np.empty_like(img)
    part = (img[..., 3] >= 128) if abin else (img[..., 3] > 0)
    cur = np.zeros((W + 2, 3), np.int64)
    nxt = np.zeros((W + 2, 3), np.int64)
    cache = {}
    for y in range(H):
        row = img[y].astype(np.int64)
        for x in range(W):
            b0, g0, r0, a = (int(v) for v in row[x])
            v = [r0, g0, b0]
            pt = bool(part[y, x]) and L > 0
            if pt:
                e = cur[x + 1]                   # stored as r, g, b
                v = [min(255, max(0, v[k] + ((int(e[k]) * L + 64) >> 7))) for k in range(3)]
            key = (v[0], v[1], v[2])
            idx = cache.get(key)
            if idx is None:
                d = ((P - np.array(v)) ** 2).sum(1)
                idx = int(np.argmin(d))
                cache[key] = idx
            q = P[idx]
            if pt:
                for k in range(3):
                    dd = v[k] - int(q[k])
                    cur[x + 2, k] += 7 * dd
                    nxt[x, k] += 3 * dd
                    nxt[x + 1, k] += 5 * dd
                    nxt[x + 2, k] += dd
            ao = a if not abin else (255 if a >= 128 else 0)
            out[y, x] = (q[2], q[1], q[0], ao)
        cur, nxt = nxt, np.zeros((W + 2, 3), np.int64)
    return out


def suite_quantize(h, rep, only, quick):
    if only and "quantize" not in only:
        return
    info = h.effects()
    if "quantize" not in info:
        rep.check(False, "quantize registered")
        return
    imgs = [("photo", img_photo(160, 120)), ("testsrc", with_alpha_hole(img_testsrc(128, 96))),
            ("few", img_gradient(64, 48) // 64 * 64)]
    cases = []
    for alg in ("octree", "median_cut"):
        for n in (2, 16, 256):
            for dith in (0, 8):
                for am in ("keep", "binary"):
                    cases.append({"colors": n, "algorithm": alg, "dithering": dith, "alpha": am})
    if quick:
        cases = cases[::5]
    specs, exp = [], []
    for p in cases:
        for iname, im in imgs:
            specs.append(Spec("quantize", im, p, tile=64))
            exp.append((f"quantize {p} on {iname}", ref_quantize(im, p)))
    for r, (name, e) in zip(h.runs(specs), exp):
        rep.compare(name, r, e, 0)
        if r.out is not None:
            ncol = len(np.unique(r.out[..., :3].reshape(-1, 3), axis=0))
            n = int(name.split("'colors': ")[1].split(",")[0])
            rep.check(ncol <= n + 1, f"{name}: {ncol} colours > {n}")
    gimg = with_alpha_hole(img_testsrc(200, 150))
    generic_checks(h, "quantize", gimg, {"colors": 16, "dithering": 8}, rep)
    generic_checks(h, "quantize", gimg, {"colors": 7, "algorithm": "median_cut"}, rep)


# ---------------------------------------------------------------- benchmark / view

BENCH = [("invert_colors", {}), ("invert_alpha", {}), ("black_white", {}), ("sepia", {}),
         ("brightness_contrast", {"brightness": 20, "contrast": 30}),
         ("posterize", {"red": 5}), ("hue_saturation", {"hue": 40, "saturation": 130, "lightness": 10}),
         ("levels", {"gamma_red": 1.5}), ("auto_level", {}), ("curves", {"luminosity": curve_blob([(0, 0), (80, 120), (255, 255)])}),
         ("curves", {"mode": "rgb", "red": curve_blob([(0, 0), (80, 120), (255, 255)])}),
         ("exposure", {"exposure": 1.0}), ("temperature_tint", {"temperature": 50, "tint": 10}),
         ("highlights_shadows", {"shadows": 60, "highlights": 40}),
         ("edge_detect", {}), ("emboss", {}), ("relief", {}), ("outline", {}),
         ("quantize", {"colors": 64, "dithering": 0}), ("quantize", {"colors": 64, "dithering": 8})]


def bench(h, rep, only):
    img = img_photo(2000, 1500)
    img = np.concatenate([img, img], 1)
    img = np.concatenate([img, img], 0)            # 4000 x 3000
    info = h.effects()
    specs = []
    for fx, p in BENCH:
        if fx in info and (not only or fx in only):
            specs.append(Spec(fx, img, p, threads=1, bench=3, keep_out=False))
            specs.append(Spec(fx, img, p, bench=3, keep_out=False))
    res = h.runs(specs)
    print(f"  [{h.target}] 4000x3000, best of 3 (Mpx/s = render only; prepare separate)")
    for i in range(0, len(res), 2):
        a, b = res[i], res[i + 1]
        print(f"    {a.fx:22s} {str(specs[i].params)[:40]:40s} 1 thread {a.mpxs:5d} Mpx/s "
              f"(prepare {a.prepare_us / 1000:7.1f} ms)   pool {b.mpxs:5d} Mpx/s   {'' if a.ok else a.line}")


def view(h, outdir, only):
    outdir = Path(outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    src = with_alpha_hole(img_photo(240, 180))
    tsrc = img_testsrc(240, 180)
    info = h.effects()
    sets = [("src", None, {})] + [(fx, fx, p) for fx, p in BENCH]
    sets += [("hs_neg", "hue_saturation", {"hue": -120, "saturation": 40}),
             ("levels_rgb", "levels", {"input_black": "302010", "gamma_red": 2.0, "gamma_blue": 0.6}),
             ("quant8", "quantize", {"colors": 8, "dithering": 8}),
             ("quant8mc", "quantize", {"colors": 8, "algorithm": "median_cut", "dithering": 0}),
             ("emboss90", "emboss", {"angle": 90}), ("outline10", "outline", {"thickness": 10, "intensity": 80})]
    sets = [s for s in sets if s[1] is None or (s[1] in info and (not only or s[1] in only))]
    for base, im in (("photo", src), ("testsrc", tsrc)):
        specs = [Spec(fx, im, p) for _, fx, p in sets if fx]
        res = h.runs(specs)
        tiles = [im]
        for r in res:
            tiles.append(r.out if r.out is not None else np.zeros_like(im))
        sheet = contact_sheet([t for t in tiles], cols=5)
        p = outdir / f"fxa_{base}.png"
        write_png(p, sheet, bg=True)
        print(f"  wrote {p}  ({', '.join(s[0] for s in sets)})")


# ---------------------------------------------------------------- main

def build_efftest(out, targets):
    for t in targets:
        r = subprocess.run([sys.executable, str(ROOT.parent / "legacy" / "build.py"), "app", "efftest", "--target", t,
                            "--out", str(out)], cwd=ROOT, capture_output=True, text=True)
        if r.returncode != 0:
            print(r.stdout[-3000:], r.stderr[-3000:])
            raise SystemExit(f"build failed ({t})")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/fxa")
    ap.add_argument("--only", default="")
    ap.add_argument("--suite", default="adjust,stylize,quantize")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--view", default="")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    if not a.no_build:
        build_efftest(out, targets)
    only = set(x for x in a.only.split(",") if x)
    suites = set(a.suite.split(","))
    total_fail = 0
    for t in targets:
        h = Harness(t, out)
        rep = Report(t)
        if a.view:
            view(h, ROOT / a.view / t, only)
        if a.bench:
            bench(h, rep, only)
        if not a.bench and not a.view:
            if "adjust" in suites:
                suite_adjust(h, rep, only, a.quick)
            if "stylize" in suites:
                suite_stylize(h, rep, only, a.quick)
            if "quantize" in suites:
                suite_quantize(h, rep, only, a.quick)
            print(f"[{t}] {rep.passed} checks passed, {len(rep.fails)} failed")
        total_fail += len(rep.fails)
        if not a.keep:
            shutil.rmtree(h.work, ignore_errors=True)
    print("fx_check:", "all passed" if total_fail == 0 else f"{total_fail} FAILED")
    return 1 if total_fail else 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    sys.exit(main())
