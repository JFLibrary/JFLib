#!/usr/bin/env python3
"""Checks of the BC1..BC7 encoders (ASMLIB/src/img/bc_enc.s, bc7_enc.s, bc6h_enc.s) through
apps/codectest (ops 107..112 in dds_ops.s, CB).

Every encoded block is decoded by the Python reference decoders of tools/dds_check.py (D3D11
functional spec, verified against Mesa by CD) and by the library's bc_dec.s; the streams are
additionally decoded by Mesa's llvmpipe (WSL, EGL + ctypes) and, for BC1..BC3, by WIC -- the
results must agree (BC6H / BC7 exactly, BC1..BC5 exactly with bc_dec.s / WIC and within 2 of
Mesa, which bit-replicates and truncates).

Suites
  block     op 107 on synthetic blocks (noise, one / two colours, ramps, extremes, alpha
            patterns) for every format x level x metric: valid streams (no reserved BC6H /
            BC7 modes), bc_dec.s == Python reference, BC1 transparency exactly at the alpha
            threshold, one-colour blocks exact (BC7) / within the table error (BC1 <= 1..2),
            BC4 / BC5 / BC3-alpha blocks with <= 2 values exact, BC2 alpha = round(a / 17);
            slow BC4 vs an exhaustive search over all 65536 endpoint pairs
  image     op 108 on photos (Windows wallpapers, when present) and synthetic images (ffmpeg
            lavfi testsrc2 / smptehdbars / mandelbrot, gradients, noise, alpha ramps, text-
            like edges): bc_dec.s decode vs input -> PSNR per format x level (table), a
            random sample of every stream also through the Python reference (identical);
            minimum PSNR per format on the photos (BC1 >= 33, BC7 normal >= 42 dB)
  mesa      the image suite's streams (samples) through Mesa: BC6H / BC7 identical to the
            reference, BC1..BC5 within 2
  ref       the same photos through independent encoders: Mesa's DXT1 / DXT5 / RGTC /
            BPTC / BPTC-float compressors and WIC's DDS encoder (DirectXTex code, BC3), PSNR
            next to ours (notes)
  edges     odd sizes 1x1, 3x5, 5x3, 4097x1, 1x4097, 17x9 (padding repeats the last column /
            row), bc_encode_to cancel flag -> ERR_CANCEL, bad format -> ERR_UNSUPPORTED
  determinism  --threads 1 vs the pool: identical bytes; Windows vs Linux: identical bytes
  perf      single thread and pool: BC1 4096^2 (every level), BC7 2048^2 (normal: < 6 s single
            thread), BC3 / BC5 / BC6H 2048^2

  python tools/bc_check.py --target both --out build/cb
  python tools/bc_check.py --target win --only block,image --no-build
  python tools/bc_check.py --tables          # print the generated tables of bc_enc.s / bc7_enc.s
"""

import os
import random
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
import dds_check as ref  # noqa: E402

C = it.C
Rec = it.Rec
SUITES = "block,image,mesa,ref,edges,determinism,perf"

OP_BCDEC = 102
OP_BCBLOCK = 100
OP_BCENC_BLK, OP_BCENC, OP_BCENC_T, OP_BCENC_CAN, OP_ENC_SYN, OP_ENC_T = range(107, 113)
BS = ref.BLOCK_BYTES
NAMES = {1: "BC1", 2: "BC2", 3: "BC3", 4: "BC4", 5: "BC5", 6: "BC6H", 7: "BC7", 8: "BC4S", 9: "BC5S",
         10: "BC6HS"}
LEVELS = {1: "fast", 2: "normal", 3: "slow"}
UNIFORM = 4


def cfg_arg(fmt, level, uniform=False, athr=0):
    return fmt | ((level | (UNIFORM if uniform else 0) | (athr << 8)) << 8)


# ================================================================== generated tables

def expand(c, e):
    return ((c << (8 - e)) | (c >> (2 * e - 8))) & 255 if e < 8 else c


def gen_tables():
    """Print the tables generated for bc_enc.s (bc1_s*) and bc7_enc.s (b7_qt, b7_single,
    b7_m2, b7_m3) and bc6h_enc.s (h6_half)."""
    def rows(vals, per=16, fmt="{}"):
        return "\n".join("        .byte   " + ", ".join(fmt.format(v) for v in vals[i:i + per])
                         for i in range(0, len(vals), per))
    out = ["# b7_qt: best effective code (e bits incl. p-bit as bit 0) for an 8-bit value x;",
           "# 12 tables of 256 bytes: e = 5, 6, 7, 8 (rows) x {any, p = 0, p = 1}; ties -> lower code",
           "b7_qt:"]
    for e in (5, 6, 7, 8):
        ex = [expand(c, e) for c in range(1 << e)]
        for p in (None, 0, 1):
            cands = [c for c in range(1 << e) if p is None or (c & 1) == p]
            out.append(f"        # e = {e}, p = {p}")
            out.append(rows([min(cands, key=lambda c: (abs(ex[c] - x), c)) for x in range(256)]))
    t = []
    for x in range(256):
        best = min((abs(((43 * expand(lo, 7) + 21 * expand(hi, 7) + 32) >> 6) - x), lo, hi)
                   for lo in range(128) for hi in range(128))
        t += [best[1], best[2]]
    out += ["b7_single:", rows(t)]
    out += ["b7_m2:"] + ["        .short  " + ", ".join(f"0x{ref.P2[p]:04X}" for p in range(i, i + 8))
                         for i in range(0, 64, 8)]
    m3 = []
    for p in range(64):
        m3 += [sum(1 << i for i in range(16) if ref.P3[p][i] == 1), sum(1 << i for i in range(16) if ref.P3[p][i] == 2)]
    out += ["b7_m3:"] + ["        .short  " + ", ".join(f"0x{v:04X}" for v in m3[i:i + 8]) for i in range(0, 128, 8)]
    t53 = [(510 * s + 93) // 186 for s in range(94)]
    t52 = [(510 * s + 61) // 124 for s in range(63)]
    t63 = [(510 * s + 189) // 378 for s in range(190)]
    t62 = [(510 * s + 125) // 252 for s in range(127)]
    for name, n, t3, t2 in (("5", 31, t53, t52), ("6", 63, t63, t62)):
        four, three = [], []
        for x in range(256):
            b4 = min((abs(t3[2 * a + b] - x), a, b) for a in range(n + 1) for b in range(n + 1))
            b3 = min((abs(t2[a + b] - x), a, b) for a in range(n + 1) for b in range(n + 1))
            four += [b4[1], b4[2], b4[0]]
            three += [b3[1], b3[2], b3[0]]
        out += [f"bc1_s4_{name}:", rows(four, 24), f"bc1_s3_{name}:", rows(three, 24)]
    h = (np.arange(256, dtype=np.float64) / 255).astype(np.float16).view(np.uint16)
    out += ["h6_half:"] + ["        .short  " + ", ".join(f"0x{v:04X}" for v in h[i:i + 8]) for i in range(0, 256, 8)]
    print("\n".join(out))


# ================================================================== reference decoding helpers

def ref_decode_blocks(fmt, data):
    """bytes of n blocks -> (n, 16, 4) RGBA (Python reference)."""
    bs = BS[fmt]
    return np.array([ref.ref_block(fmt, data[i:i + bs]) for i in range(0, len(data), bs)], np.int32).reshape(-1, 16, 4)


def asm_decode_blocks_rgba(raw):
    """op 100 output (64 bytes BGRA per block) -> (n, 16, 4) RGBA."""
    a = np.frombuffer(raw, np.uint8).reshape(-1, 16, 4).astype(np.int32)
    return a[..., [2, 1, 0, 3]]


def block_bytes(px):
    """(16, 4) RGBA -> 64 bytes straight BGRA."""
    return np.ascontiguousarray(np.asarray(px, np.uint8)[:, [2, 1, 0, 3]]).tobytes()


def lum(bgra):
    b, g, r = (bgra[..., i].astype(np.int64) for i in range(3))
    return ((7471 * b + 38470 * g + 19595 * r) >> 16).astype(np.int64)


def target_of(fmt, rgba):
    """What the decoder should reproduce for an RGBA (..., 4) input: (target, decoded channel
    selection) as RGBA arrays with don't-care channels equal."""
    rgba = np.asarray(rgba, np.int64)
    if fmt in (4, 8):
        y = lum(rgba[..., [2, 1, 0, 3]])
        return np.stack([y, y, y, np.full_like(y, 255)], -1)
    if fmt in (5, 9):
        return np.stack([rgba[..., 0], rgba[..., 1], np.zeros_like(rgba[..., 0]), np.full_like(rgba[..., 0], 255)], -1)
    if fmt in (6, 10):
        out = rgba.copy()
        out[..., 3] = 255
        return out
    if fmt == 1:
        return rgba
    return rgba


# ================================================================== test blocks

def gen_test_blocks(rng, n_noise=40):
    """-> list of (name, (16, 4) RGBA uint8)."""
    nr = np.random.default_rng(rng.randrange(1 << 30))
    out = []
    for _ in range(n_noise):
        b = nr.integers(0, 256, (16, 4))
        b[:, 3] = 255
        out.append(("noise", b))
    for _ in range(n_noise // 2):
        out.append(("noise-alpha", nr.integers(0, 256, (16, 4))))
    for _ in range(24):
        c = nr.integers(0, 256, 4)
        if rng.random() < 0.6:
            c[3] = 255
        out.append(("constant", np.tile(c, (16, 1))))
    for c in ((0, 0, 0, 255), (255, 255, 255, 255), (0, 0, 0, 0), (255, 255, 255, 0), (128, 128, 128, 255),
              (1, 2, 3, 255), (254, 253, 252, 255)):
        out.append(("constant-x", np.tile(np.array(c), (16, 1))))
    for _ in range(20):
        a, b = nr.integers(0, 256, 4), nr.integers(0, 256, 4)
        a[3] = b[3] = 255
        sel = nr.integers(0, 2, 16)
        out.append(("two-colour", np.where(sel[:, None] == 1, b, a)))
    for _ in range(20):
        a, b = nr.integers(0, 256, 4).astype(float), nr.integers(0, 256, 4).astype(float)
        a[3] = b[3] = 255
        d = nr.normal(size=2)
        t = np.array([(x * d[0] + y * d[1]) for y in range(4) for x in range(4)])
        t = (t - t.min()) / max(1e-9, t.max() - t.min())
        out.append(("ramp", np.rint(a + (b - a) * t[:, None]).astype(int)))
    for _ in range(12):
        b = nr.integers(0, 256, (16, 4))
        b[:, 3] = nr.choice([0, 255, 100, 127, 128, 200], 16)
        out.append(("alpha-mix", b))
    b = np.zeros((16, 4), int)
    b[:, :3] = np.array([0, 255] * 8)[:, None]
    b[:, 3] = 255
    out.append(("stripes", b))
    b = np.full((16, 4), 30)
    b[5] = (250, 10, 200, 255)
    b[:, 3] = 255
    out.append(("outlier", b))
    b = np.zeros((16, 4), int)
    b[:, 0] = np.arange(16) * 17
    b[:, 3] = np.arange(16) * 17
    out.append(("alpha-ramp", b))
    return [(n, np.clip(b, 0, 255).astype(np.uint8)) for n, b in out]


def bc4_optimal(vals, signed):
    """Exhaustive BC4 search over all endpoint pairs -> smallest squared error (bytes)."""
    vals = np.asarray(vals, np.int64)
    best = None
    rng_ = range(-127, 128) if signed else range(256)
    for e0 in rng_:
        pals = []
        for e1 in rng_:
            blk = bytes([e0 & 255, e1 & 255]) + bytes(6)
            pals.append(ref.bc4_vals(blk, signed))
        pals = np.array(pals, np.int64)                         # (256, 8)
        d = ((pals[:, None, :] - vals[None, :, None]) ** 2).min(-1).sum(-1)
        m = int(d.min())
        best = m if best is None else min(best, m)
    return best


def suite_block(runner, suite, rng, quick=False):
    blocks = gen_test_blocks(rng)
    data = b"".join(block_bytes(b) for _, b in blocks)
    recs, jobs = [], []
    for fmt in range(1, 11):
        for level in (1, 2, 3):
            for uni in ((False, True) if fmt in (1, 2, 3) else (False,)):
                athrs = (0, 1, 201) if fmt == 1 and not uni else (0,)
                for athr in athrs:
                    arg = cfg_arg(fmt, level, uni, athr)
                    jobs.append((fmt, level, uni, athr))
                    recs.append(Rec(OP_BCENC_BLK, arg, data, None, f"{NAMES[fmt]} {LEVELS[level]} uni={uni} athr={athr}"))
    res = runner.batch(recs)
    # decode everything with bc_dec.s (op 100) in one batch
    dec_recs = []
    for (fmt, level, uni, athr), r in zip(jobs, res):
        if r is not None and r[0] == 0:
            dec_recs.append(Rec(OP_BCBLOCK, fmt, r[1], None, "dec"))
    dres = iter(runner.batch(dec_recs))
    notes = {}
    for (fmt, level, uni, athr), r in zip(jobs, res):
        name = f"{NAMES[fmt]} {LEVELS[level]}{' uniform' if uni else ''}{f' athr={athr}' if athr else ''}"
        if r is None or r[0] != 0:
            suite.fail(f"{name}: encode {'crash' if r is None else it.err_name(r[0])}")
            continue
        enc = r[1]
        if len(enc) != len(blocks) * BS[fmt]:
            suite.fail(f"{name}: {len(enc)} bytes")
            continue
        d = next(dres)
        pyd = ref_decode_blocks(fmt, enc)
        if d is None or d[0] != 0:
            suite.fail(f"{name}: bc_decode_block failed")
            continue
        asd = asm_decode_blocks_rgba(d[1])
        suite.check(np.array_equal(asd, pyd), f"{name}: bc_dec.s differs from the Python reference")
        thr = 128 if athr == 0 else athr - 1
        errs = {}
        for i, (kind, px) in enumerate(blocks):
            blk = enc[i * BS[fmt]:(i + 1) * BS[fmt]]
            if fmt == 7:
                suite.check(blk[0] != 0, f"{name} {kind}: reserved BC7 mode")
            if fmt in (6, 10):
                mv = blk[0] & 3 if blk[0] & 3 < 2 else blk[0] & 31
                suite.check(mv not in ref.BC6_RESERVED, f"{name} {kind}: reserved BC6H mode")
            dec = pyd[i]
            tgt = target_of(fmt, px)
            if fmt == 1:
                tr = px[:, 3] < thr
                ok = np.all(dec[tr, 3] == 0) and np.all(dec[~tr, 3] == 255)
                suite.check(ok, f"{name} {kind}: transparency does not follow the threshold {thr}")
                cmpc = [0, 1, 2]
                sel = ~tr
            elif fmt in (6, 10):
                cmpc, sel = [0, 1, 2], np.ones(16, bool)
            elif fmt in (4, 8):
                cmpc, sel = [0], np.ones(16, bool)
            elif fmt in (5, 9):
                cmpc, sel = [0, 1], np.ones(16, bool)
            else:
                cmpc, sel = [0, 1, 2, 3], np.ones(16, bool)
            diff = np.abs(dec[sel][:, cmpc] - tgt[sel][:, cmpc])
            mx = int(diff.max()) if diff.size else 0
            errs.setdefault(kind, []).append(float((diff.astype(np.int64) ** 2).sum()))
            if kind.startswith("constant"):
                lim = {1: 2, 2: 2, 3: 2, 7: 0, 4: 0, 5: 0, 8: 1, 9: 1, 6: 20, 10: 20}[fmt]
                if fmt in (2, 3) and px[0, 3] != 255:
                    lim = max(lim, 8 if fmt == 2 else 0)
                if fmt == 2:
                    lim = 8
                suite.check(mx <= lim, f"{name} {kind} {px[0]}: max |d| {mx} > {lim}")
            if kind == "two-colour" and fmt in (3, 4, 5):
                m = int(np.abs(dec[:, 3] - tgt[:, 3]).max()) if fmt == 3 else mx
                suite.check(m == 0, f"{name} {kind}: not exact ({m})")
            if fmt == 2:
                a_exp = ((px[:, 3].astype(int) + 8) // 17) * 17
                suite.check(np.array_equal(dec[:, 3], a_exp), f"{name} {kind}: BC2 alpha not round(a / 17)")
        for kind, e in errs.items():
            notes.setdefault(kind, {})[name] = sum(e) / len(e)
    for kind in ("noise", "ramp", "two-colour"):
        if kind in notes:
            items = [f"{k} {v:.0f}" for k, v in notes[kind].items() if "athr" not in k]
            suite.notes.append(f"mean block SSE {kind}: " + ", ".join(items[:12]))
    # BC4 slow vs exhaustive (a few blocks)
    if not quick:
        nr = np.random.default_rng(rng.randrange(1 << 30))
        tests = [nr.integers(0, 256, 16) for _ in range(3)] + [np.clip(nr.normal(128, 30, 16), 0, 255).astype(int) for _ in range(3)]
        recs = []
        for signed in (False, True):
            px = [np.stack([v, v, v, np.full(16, 255)], -1) for v in tests]
            data = b"".join(block_bytes(p) for p in px)
            recs.append(Rec(OP_BCENC_BLK, cfg_arg(8 if signed else 4, 3), data, None, "bc4 slow"))
        rr = runner.batch(recs)
        worst = {"noise": 1.0, "smooth": 1.0}
        for signed, r in zip((False, True), rr):
            if r is None or r[0] != 0:
                suite.fail("bc4 exhaustive: encode failed")
                continue
            for i, v in enumerate(tests):
                blk = r[1][i * 8:(i + 1) * 8]
                got = sum((a - b) ** 2 for a, b in zip(ref.bc4_chan(blk, signed), (lum(np.stack([v, v, v], -1)))))
                opt = bc4_optimal(lum(np.stack([v, v, v], -1)), signed)
                ratio = got / opt if opt else (1.0 if got == 0 else 99)
                kind = "noise" if i < 3 else "smooth"
                worst[kind] = max(worst[kind], ratio)
                lim = 1.35 if kind == "noise" else 1.05
                suite.check(ratio <= lim, f"BC4{'S' if signed else ''} slow {kind} SSE {got} vs optimal {opt}")
        suite.notes.append("BC4 slow vs exhaustive search: worst SSE ratio " +
                           ", ".join(f"{k} {v:.3f}" for k, v in worst.items()))


# ================================================================== test images

WALLPAPERS = [("photo-lake", r"C:\Windows\Web\Wallpaper\ThemeC\img29.jpg"),
              ("photo-hills", r"C:\Windows\Web\Wallpaper\ThemeC\img28.jpg"),
              ("render-stripes", r"C:\Windows\Web\Wallpaper\ThemeB\img26.jpg"),
              ("render-ribbons", r"C:\Windows\Web\Wallpaper\ThemeB\img25.jpg"),
              ("photo-collage", r"C:\Windows\Web\Wallpaper\Spotlight\img50.jpg")]


def ffmpeg_bgra(args, w, h):
    p = subprocess.run(["ffmpeg", "-v", "error", *args, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "bgra", "-"],
                       capture_output=True, timeout=300)
    if p.returncode != 0 or len(p.stdout) < w * h * 4:
        return None
    return np.frombuffer(p.stdout, np.uint8, count=w * h * 4).reshape(h, w, 4).copy()


def photos(size=512):
    out = []
    if not it.have_exe("ffmpeg"):
        return out
    for name, path in WALLPAPERS:
        if not os.path.exists(path):
            continue
        img = ffmpeg_bgra(["-i", path, "-vf", f"scale=960:600,crop={size}:{size}:{(960 - size) // 2}:{(600 - size) // 2}"],
                          size, size)
        if img is not None:
            out.append((name, img))
    return out


def synthetic_images(rng, size=256):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    out = []
    if it.have_exe("ffmpeg"):
        for name, src in (("testsrc2", "testsrc2"), ("smptehdbars", "smptehdbars"), ("mandelbrot", "mandelbrot")):
            img = ffmpeg_bgra(["-f", "lavfi", "-i", f"{src}=size={size}x{size}"], size, size)
            if img is not None:
                img[..., 3] = 255
                out.append((name, img))
    y, x = np.mgrid[0:size, 0:size].astype(np.float64)
    g = np.zeros((size, size, 4), np.uint8)
    g[..., 0] = np.rint(x / (size - 1) * 255)
    g[..., 1] = np.rint(y / (size - 1) * 255)
    g[..., 2] = np.rint((x + y) / (2 * size - 2) * 255)
    g[..., 3] = 255
    out.append(("gradient", g))
    n = nr.integers(0, 256, (size, size, 4), dtype=np.uint8)
    n[..., 3] = 255
    out.append(("noise", n))
    a = g.copy()
    a[..., 3] = np.rint(x / (size - 1) * 255).astype(np.uint8)
    a[size // 4: size // 2, :, 3] = 0
    out.append(("alpha-ramp", a))
    t = np.full((size, size, 4), 255, np.uint8)
    for k in range(0, size, 11):
        t[k:k + 2, 5:size - 5, :3] = 0                          # text-like horizontal strokes
        t[5:size - 5, (k * 7) % size, :3] = (20, 20, 180)
    for k in range(0, size, 23):
        t[k:k + 9, k:k + 3, :3] = (0, 120, 0)
    out.append(("text-edges", t))
    return out


def psnr_fmt(fmt, dec, src):
    """PSNR of the channels the format carries (opaque sources: RGB; BC2 / BC3 / BC7 with
    alpha: RGBA; BC1 with alpha: RGB of the opaque pixels + the 1-bit alpha)."""
    dec = dec.astype(np.int64)
    src = src.astype(np.int64)
    if fmt in (4, 8):
        return it.psnr(dec[..., 2], lum(src))
    if fmt in (5, 9):
        return it.psnr(dec[..., 1:3], src[..., 1:3])
    opaque = bool((src[..., 3] == 255).all())
    if fmt in (6, 10) or opaque:
        return it.psnr(dec[..., :3], src[..., :3])
    if fmt == 1:
        vis = src[..., 3] >= 128
        return it.psnr(dec[vis][:, :3], src[vis][:, :3]) if vis.any() else 99.0
    return it.psnr(dec, src)


def encode_images(runner, images, fmts, levels, extra_cfg=0):
    """-> dict (name, fmt, level) -> (blocks bytes or None, error)."""
    recs, keys = [], []
    for name, img in images:
        surf = it.surface_input(img, C.FMT_ARGB)
        for fmt in fmts:
            for level in levels:
                recs.append(Rec(OP_BCENC, cfg_arg(fmt, level) | (extra_cfg << 8), surf, None, f"{name} {fmt} {level}"))
                keys.append((name, fmt, level))
    res = runner.batch(recs)
    return {k: (None if r is None or r[0] != 0 else r[1], None if r is None else r[0]) for k, r in zip(keys, res)}


def decode_images(runner, images, enc):
    """op 102 (bc_decode) of every stream -> dict key -> (h, w, 4) BGRA."""
    dims = {name: img.shape[:2] for name, img in images}
    recs, keys = [], []
    for k, (data, _) in enc.items():
        if data is None:
            continue
        h, w = dims[k[0]]
        recs.append(Rec(OP_BCDEC, k[1], struct.pack("<II", w, h) + data, None, "dec"))
        keys.append(k)
    out = {}
    for k, r in zip(keys, runner.batch(recs)):
        if r is not None and r[0] == 0:
            out[k] = it.parse_surface(r[1])[4]
    return out


PSNR_MIN = {  # (fmt, level) minimum on the photos
    (1, 1): 32, (1, 2): 33, (1, 3): 33, (2, 2): 33, (3, 2): 33, (7, 1): 36, (7, 2): 42, (7, 3): 42,
    (4, 2): 40, (5, 2): 40, (6, 2): 36}


def suite_image(runner, suite, rng, state, quick=False):
    ph = photos()
    syn = synthetic_images(rng)
    if not ph:
        suite.notes.append("no photos (Windows wallpapers / ffmpeg) -- photo thresholds skipped")
    images = ph + syn
    fmts = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    levels = [1, 2, 3] if not quick else [2]
    enc = encode_images(runner, images, fmts, levels)
    dec = decode_images(runner, images, enc)
    state["images"], state["enc"] = images, enc
    table = {}
    nr = random.Random(rng.randrange(1 << 30))
    for (name, fmt, level), (data, err) in enc.items():
        key = (name, fmt, level)
        if data is None:
            suite.fail(f"{name} {NAMES[fmt]} {LEVELS[level]}: {it.err_name(err) if err is not None else 'crash'}")
            continue
        img = dict(images)[name]
        h, w = img.shape[:2]
        if key not in dec:
            suite.fail(f"{name} {NAMES[fmt]}: bc_decode failed")
            continue
        d = dec[key]
        # sample through the Python reference
        bw = (w + 3) // 4
        nb = len(data) // BS[fmt]
        ok = True
        for b in nr.sample(range(nb), min(24, nb)):
            px = ref.ref_block(fmt, data[b * BS[fmt]:(b + 1) * BS[fmt]])
            by, bx = divmod(b, bw)
            for i, (r_, g_, b_, a_) in enumerate(px):
                y, x = by * 4 + i // 4, bx * 4 + i % 4
                if y < h and x < w and tuple(d[y, x]) != (b_, g_, r_, a_):
                    ok = False
        suite.check(ok, f"{name} {NAMES[fmt]} {LEVELS[level]}: bc_dec.s != Python reference")
        if fmt == 7:
            suite.check(all(data[i] != 0 for i in range(0, len(data), 16)), f"{name} BC7: reserved mode")
        p = psnr_fmt(fmt, d, img)
        table[key] = p
        if name.startswith(("photo", "render")) and (fmt, level) in PSNR_MIN:
            lim = PSNR_MIN[(fmt, level)]
            suite.check(p >= lim, f"{name} {NAMES[fmt]} {LEVELS[level]}: PSNR {p:.2f} < {lim}")
    state["psnr"] = table
    for name, _ in images:
        row = []
        for fmt in fmts:
            vals = [table.get((name, fmt, lv)) for lv in levels]
            row.append(f"{NAMES[fmt]} " + "/".join("-" if v is None else f"{v:.1f}" for v in vals))
        suite.notes.append(f"PSNR {name:15s} " + "  ".join(row))
    # uniform metric (BC1 / BC3) on the photos
    if ph:
        encu = encode_images(runner, ph, [1, 3], [2], extra_cfg=UNIFORM)
        decu = decode_images(runner, ph, encu)
        for (name, fmt, level), d in decu.items():
            suite.notes.append(f"uniform metric {name} {NAMES[fmt]} normal: PSNR {psnr_fmt(fmt, d, dict(ph)[name]):.2f}")


# ================================================================== Mesa cross-check

def suite_mesa(suite, rng, state):
    enc = state.get("enc")
    if not enc:
        suite.notes.append("image suite not run -- skipped")
        return
    nr = random.Random(rng.randrange(1 << 30))
    jobs = {}
    for (name, fmt, level), (data, _) in enc.items():
        if data is None:
            continue
        nb = len(data) // BS[fmt]
        for b in nr.sample(range(nb), min(40, nb)):
            jobs.setdefault(fmt, []).append(data[b * BS[fmt]:(b + 1) * BS[fmt]])
    order = sorted(jobs)
    res = ref.mesa_decode([(f, jobs[f]) for f in order])
    if res is None:
        suite.notes.append("Mesa (WSL llvmpipe) not available -- skipped")
        return
    for fmt, raw in zip(order, res):
        exact = worst = 0
        for n, b in enumerate(jobs[fmt]):
            if fmt in (6, 10):
                r = ref.bc6_block_half(b, fmt == 10)
                g = [struct.unpack_from("<3H", raw, (n * 16 + i) * 6) for i in range(16)]
                ok = all((x == y) or (x & 0x7FFF == 0 and y & 0x7FFF == 0) for p, q in zip(r, g) for x, y in zip(p, q))
                suite.check(ok, f"mesa {NAMES[fmt]} block {b.hex()}")
                exact += ok
                continue
            r = ref.ref_block(fmt, b)
            if fmt in (4, 8, 5, 9):
                nc = 1 if fmt in (4, 8) else 2
                vals = struct.unpack_from(f"<{16 * nc}f", raw, n * 16 * 4 * nc)
                g = []
                for i in range(16):
                    v = [vals[i * nc + c] for c in range(nc)]
                    u = [int(np.floor(((x + 1) / 2 if fmt in (8, 9) else x) * 255 + 0.5)) for x in v]
                    g.append((u[0], u[0], u[0], 255) if nc == 1 else (u[0], u[1], 0, 255))
            else:
                g = [tuple(raw[(n * 16 + i) * 4:(n * 16 + i) * 4 + 4]) for i in range(16)]
            d = max(abs(x - y) for p, q in zip(r, g) for x, y in zip(p, q))
            worst = max(worst, d)
            exact += d == 0
            suite.check(d == 0 if fmt == 7 else d <= 2, f"mesa {NAMES[fmt]} block {b.hex()}: |d| {d}")
        suite.notes.append(f"{NAMES[fmt]}: {exact}/{len(jobs[fmt])} sampled blocks identical to Mesa, max |d| {worst}")


# ================================================================== reference encoders

MESA_ENC_CODE = r'''
import ctypes, os, struct, sys
os.environ["LIBGL_ALWAYS_SOFTWARE"] = "1"
os.environ["GALLIUM_DRIVER"] = "llvmpipe"
egl = ctypes.CDLL("libEGL.so.1")
egl.eglGetProcAddress.restype = ctypes.c_void_p
egl.eglGetProcAddress.argtypes = [ctypes.c_char_p]
gpd = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)(egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
dpy = gpd(0x31DD, None, None)
egl.eglInitialize.argtypes = [ctypes.c_void_p] * 3
if not egl.eglInitialize(dpy, None, None):
    sys.exit(3)
egl.eglBindAPI(0x30A2)
egl.eglCreateContext.restype = ctypes.c_void_p
egl.eglCreateContext.argtypes = [ctypes.c_void_p] * 4
ctx = egl.eglCreateContext(dpy, None, None, None)
egl.eglMakeCurrent.argtypes = [ctypes.c_void_p] * 4
egl.eglMakeCurrent(dpy, None, None, ctx)
def gp(name, res, *args):
    return ctypes.CFUNCTYPE(res, *args)(egl.eglGetProcAddress(name))
u, i, p = ctypes.c_uint, ctypes.c_int, ctypes.c_void_p
glGenTextures = gp(b"glGenTextures", None, i, p)
glBindTexture = gp(b"glBindTexture", None, u, u)
glTexImage2D = gp(b"glTexImage2D", None, u, i, i, i, i, i, u, u, p)
glGetCompressedTexImage = gp(b"glGetCompressedTexImage", None, u, i, p)
glGetTexLevelParameteriv = gp(b"glGetTexLevelParameteriv", None, u, i, u, p)
glGetError = gp(b"glGetError", u)
glPixelStorei = gp(b"glPixelStorei", None, u, i)
glPixelStorei(0x0CF5, 1)
glPixelStorei(0x0D05, 1)
data = sys.stdin.buffer.read()
out = bytearray()
pos = 0
while pos < len(data):
    glfmt, w, h = struct.unpack_from("<3I", data, pos)
    pos += 12
    px = data[pos:pos + w * h * 4]
    pos += w * h * 4
    tex = ctypes.c_uint()
    glGenTextures(1, ctypes.byref(tex))
    glBindTexture(0x0DE1, tex.value)
    buf = ctypes.create_string_buffer(px, len(px))
    glTexImage2D(0x0DE1, 0, glfmt, w, h, 0, 0x1908, 0x1401, buf)
    sz = ctypes.c_int()
    glGetTexLevelParameteriv(0x0DE1, 0, 0x86A0, ctypes.byref(sz))
    ob = ctypes.create_string_buffer(sz.value)
    glGetCompressedTexImage(0x0DE1, 0, ob)
    if glGetError():
        sys.exit(1)
    out += struct.pack("<I", sz.value) + ob.raw
sys.stdout.buffer.write(bytes(out))
'''
MESA_ENC = {1: 0x83F1, 3: 0x83F3, 4: 0x8DBB, 5: 0x8DBD, 7: 0x8E8C, 6: 0x8E8F}


def mesa_encode(jobs):
    """jobs: list of (fmt, rgba (h, w, 4)) -> list of block bytes or None."""
    parts = []
    for fmt, rgba in jobs:
        h, w = rgba.shape[:2]
        parts.append(struct.pack("<3I", MESA_ENC[fmt], w, h) + np.ascontiguousarray(rgba, np.uint8).tobytes())
    try:
        p = it.wsl_python(MESA_ENC_CODE, b"".join(parts), timeout=1800)
    except Exception:  # noqa: BLE001
        return None
    if p.returncode != 0:
        return None
    out, pos = [], 0
    for _ in jobs:
        n = struct.unpack_from("<I", p.stdout, pos)[0]
        out.append(p.stdout[pos + 4:pos + 4 + n])
        pos += 4 + n
    return out


def suite_ref(runner, suite, rng, state):
    table = state.get("psnr", {})
    ph = [x for x in state.get("images", []) if x[0].startswith(("photo", "render"))]
    if not ph:
        suite.notes.append("no photos -- skipped")
        return
    def mesa_input(fmt, img):
        rgba = img[..., [2, 1, 0, 3]].copy()
        if fmt == 4:                            # BC4 of the luminance, like ours
            y = lum(img).astype(np.uint8)
            rgba[..., 0] = rgba[..., 1] = rgba[..., 2] = y
        return rgba
    jobs = [(fmt, mesa_input(fmt, img)) for _, img in ph for fmt in (1, 3, 4, 5, 7, 6)]
    res = mesa_encode(jobs)
    if res is None:
        suite.notes.append("Mesa (WSL) not available")
    else:
        recs, keys = [], []
        for (fmt, rgba), data in zip(jobs, res):
            h, w = rgba.shape[:2]
            recs.append(Rec(OP_BCDEC, fmt, struct.pack("<II", w, h) + data, None, "dec"))
        k = 0
        for (name, img) in ph:
            row = []
            for fmt in (1, 3, 4, 5, 7, 6):
                r = runner.batch([recs[k]])[0]
                k += 1
                if r is None or r[0] != 0:
                    row.append(f"{NAMES[fmt]} -")
                    continue
                p = psnr_fmt(fmt, it.parse_surface(r[1])[4], img)
                ours = table.get((name, fmt, 2))
                row.append(f"{NAMES[fmt]} {p:.1f} (ours {ours:.1f})" if ours else f"{NAMES[fmt]} {p:.1f}")
                if ours is not None:
                    suite.check(ours >= p - 0.5, f"{name} {NAMES[fmt]}: ours {ours:.2f} < Mesa {p:.2f} - 0.5")
            suite.notes.append(f"Mesa encoders {name}: " + ", ".join(row))
    if runner.target == "win":
        work = Path(tempfile.mkdtemp(prefix="bcwic_", dir=runner.work))
        # WPF has no DDS encoder class: wic_dds_encode creates it from the container GUID
        res = wic_dds_encode(work, ph)
        for (name, img), data in zip(ph, res):
            if data is None:
                suite.notes.append(f"WIC DDS encoder: {name} failed")
                continue
            r = runner.one(it.OP_DECEX, 0, it.decex_input(data, C.IMGD_STRAIGHT))
            if r[0] != 0:
                suite.notes.append(f"WIC DDS {name}: our reader {it.err_name(r[0])}")
                continue
            d = it.parse_decex(r[1])
            p = psnr_fmt(3, d["px"], img)
            ours = table.get((name, 3, 2))
            suite.notes.append(f"WIC (DirectXTex BC code) BC3 {name}: PSNR {p:.2f} (ours normal {ours:.2f})")
            if ours is not None:
                suite.check(ours >= p - 0.3, f"{name} BC3: ours {ours:.2f} < WIC {p:.2f} - 0.3")


WIC_DDS_PS = r"""
param([string]$jobs)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase
$g = [Guid]'9967cb95-2e85-4ac8-8ca2-83d7ccd425c9'
foreach ($line in [IO.File]::ReadAllLines($jobs)) {
  $a = $line.Split("`t")
  try {
    $w = [int]$a[1]; $h = [int]$a[2]; $st = $w * 4
    $raw = [IO.File]::ReadAllBytes($a[0])
    $src = [System.Windows.Media.Imaging.BitmapSource]::Create($w, $h, 96, 96, [System.Windows.Media.PixelFormats]::Bgra32, $null, $raw, $st)
    $enc = [System.Windows.Media.Imaging.BitmapEncoder]::Create($g)
    $enc.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($src))
    $fs = [IO.File]::Create($a[3]); $enc.Save($fs); $fs.Close()
  } catch { }
}
"""


def wic_dds_encode(work, images):
    """WIC's DDS encoder (its default output: BC3 / DXT5) -> list of file bytes or None."""
    work = Path(work)
    ps = work / "wic_dds.ps1"
    ps.write_text(WIC_DDS_PS, encoding="utf-8")
    lines, outs = [], []
    for k, (name, img) in enumerate(images):
        raw = work / f"in{k}.raw"
        raw.write_bytes(np.ascontiguousarray(img, np.uint8).tobytes())
        out = work / f"out{k}.dds"
        if out.exists():
            out.unlink()
        lines.append(f"{raw.resolve()}\t{img.shape[1]}\t{img.shape[0]}\t{out.resolve()}")
        outs.append(out)
    jf = work / "jobs.txt"
    jf.write_text("\n".join(lines) + "\n", encoding="utf-8")
    subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(ps), str(jf)],
                   capture_output=True, timeout=1800)
    return [o.read_bytes() if o.exists() else None for o in outs]


# ================================================================== edges, cancel, determinism

def suite_edges(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    recs = []
    for w, h in ((1, 1), (3, 5), (5, 3), (4097, 1), (1, 4097), (17, 9), (4, 4), (8, 1)):
        img = nr.integers(0, 256, (h, w, 4), dtype=np.uint8)
        for fmt in (1, 3, 5, 7, 6):
            surf = it.surface_input(img, C.FMT_ARGB)

            def chk(res, out, img=img, fmt=fmt, w=w, h=h):
                if res != 0:
                    return f"error {it.err_name(res)}"
                bw, bh = (w + 3) // 4, (h + 3) // 4
                if len(out) != bw * bh * BS[fmt]:
                    return f"{len(out)} bytes, expected {bw * bh * BS[fmt]}"
                # padding repeats the last column / row: encode the padded image as one of
                # full blocks -> must give the same blocks
                return None
            recs.append(Rec(OP_BCENC, cfg_arg(fmt, 2), surf, chk, f"bc_encode {w}x{h} {NAMES[fmt]}"))
            # the same with the image padded explicitly: identical blocks
            pw, ph = (w + 3) // 4 * 4, (h + 3) // 4 * 4
            pad = np.pad(img, ((0, ph - h), (0, pw - w), (0, 0)), mode="edge")
            recs.append(Rec(OP_BCENC, cfg_arg(fmt, 2), it.surface_input(pad, C.FMT_ARGB), None, f"padded {w}x{h} {fmt}"))
    res = runner.batch(recs)
    for i in range(0, len(recs), 2):
        a, b = res[i], res[i + 1]
        r = recs[i]
        msg = r.check(*a) if a is not None else "crash"
        if msg:
            suite.fail(f"{r.name}: {msg}")
        else:
            suite.check(b is not None and a[1] == b[1], f"{r.name}: differs from the explicitly padded image")
    # other surface formats: PARGB / XRGB / A8 sources give the straight pixels
    img = nr.integers(0, 256, (8, 12, 4), dtype=np.uint8)
    pm = it.premul(img)
    recs = [Rec(OP_BCENC, cfg_arg(7, 2), it.surface_input(img, C.FMT_ARGB), None, "argb"),
            Rec(OP_BCENC, cfg_arg(7, 2), it.surface_input(pm, C.FMT_PARGB), None, "pargb")]
    x = img.copy()
    x[..., 3] = 0
    o = img.copy()
    o[..., 3] = 255
    recs += [Rec(OP_BCENC, cfg_arg(7, 2), it.surface_input(x, C.FMT_XRGB), None, "xrgb"),
             Rec(OP_BCENC, cfg_arg(7, 2), it.surface_input(o, C.FMT_ARGB), None, "opaque")]
    r = runner.batch(recs)
    unp = it.unpremul(pm)
    rr = runner.one(OP_BCENC, cfg_arg(7, 2), it.surface_input(unp, C.FMT_ARGB))
    suite.check(r[1] is not None and r[1][1] == rr[1], "PARGB source != its unpremultiplied ARGB")
    suite.check(r[2] is not None and r[2][1] == r[3][1], "XRGB source != opaque ARGB")
    # cancel, bad format, empty surface
    img = nr.integers(0, 256, (64, 64, 4), dtype=np.uint8)
    for fmt in (1, 7):
        recs = [Rec(OP_BCENC_CAN, cfg_arg(fmt, 2), it.surface_input(img, C.FMT_ARGB), it.expect_err(C.ERR_CANCEL),
                    f"cancel {NAMES[fmt]}")]
        it.run_suite(runner, suite, recs)
    it.run_suite(runner, suite, [
        Rec(OP_BCENC, 11, it.surface_input(img, C.FMT_ARGB), it.expect_err(C.ERR_UNSUPPORTED), "bcfmt 11"),
        Rec(OP_BCENC, 0, it.surface_input(img, C.FMT_ARGB), it.expect_err(C.ERR_UNSUPPORTED), "bcfmt 0"),
        Rec(OP_BCENC_BLK, 12, bytes(64), it.expect_err(C.ERR_UNSUPPORTED), "block bcfmt 12")])


def run_determinism(suite, targets, out, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    ph = photos(256)
    imgs = [img for _, img in ph[:2]] or [nr.integers(0, 256, (256, 256, 4), dtype=np.uint8)]
    a = nr.integers(0, 256, (96, 160, 4), dtype=np.uint8)
    imgs.append(a)
    recs = []
    for img in imgs:
        for fmt in range(1, 11):
            for level in (1, 2, 3):
                recs.append(Rec(OP_BCENC, cfg_arg(fmt, level), it.surface_input(img, C.FMT_ARGB), None, "det"))
    digests = {}
    for target in targets:
        for threads in ("1", "0"):
            extra = ["--threads", threads] if threads != "0" else []
            r = it.Runner(target, out, work_name="bcdet", extra=extra)
            res = r.batch(recs)
            dig = [None if x is None else zlib.crc32(x[1]) for x in res]
            digests[(target, threads)] = dig
    base = None
    for k, d in digests.items():
        if base is None:
            base = (k, d)
            continue
        same = sum(1 for x, y in zip(base[1], d) if x == y and x is not None)
        suite.check(same == len(d), f"{k} differs from {base[0]}: {len(d) - same} of {len(d)} streams")
    suite.notes.append(f"{len(recs)} streams compared across {sorted(digests)}")


# ================================================================== performance

def run_perf(suite, runner1, runnerN, rng):
    ph = photos(512)
    if ph:
        base = ph[0][1]
    else:
        base = np.random.default_rng(1).integers(0, 256, (512, 512, 4), dtype=np.uint8)
        base[..., 3] = 255

    def tile(n):
        reps = (n + base.shape[0] - 1) // base.shape[0]
        return np.tile(base, (reps, reps, 1))[:n, :n]

    def timed(r, fmt, level, img, reps=1):
        res = r.one(OP_BCENC_T, cfg_arg(fmt, level), struct.pack("<I", reps) + it.surface_input(img, C.FMT_ARGB))
        if res[0] != 0:
            return None
        return struct.unpack_from("<I", res[1])[0] / 1e6

    i4 = tile(4096)
    for level in (1, 2, 3):
        t1 = timed(runner1, 1, level, i4)
        tn = timed(runnerN, 1, level, i4)
        suite.notes.append(f"BC1 4096^2 {LEVELS[level]}: {t1:.2f} s single thread, {tn:.2f} s pool")
        if level == 2:
            suite.check(t1 is not None and t1 < 1.5, f"BC1 4096^2 normal {t1} s >= 1.5 s (single thread)")
    i2 = tile(2048)
    for level in (1, 2, 3):
        t1 = timed(runner1, 7, level, i2) if level < 3 else None
        tn = timed(runnerN, 7, level, i2)
        suite.notes.append(f"BC7 2048^2 {LEVELS[level]}: " + (f"{t1:.2f} s single thread, " if t1 else "") + f"{tn:.2f} s pool")
        if level == 2:
            suite.check(t1 is not None and t1 < 6.0, f"BC7 2048^2 normal {t1} s >= 6 s (single thread)")
    for fmt in (3, 5, 4, 6):
        t1 = timed(runner1, fmt, 2, i2)
        tn = timed(runnerN, fmt, 2, i2)
        suite.notes.append(f"{NAMES[fmt]} 2048^2 normal: {t1:.2f} s single thread, {tn:.2f} s pool")


# ================================================================== main

def main():
    if "--tables" in sys.argv:
        gen_tables()
        return 0
    ap = it.argparser(__doc__, SUITES)
    ap.add_argument("--quick", action="store_true", help="fewer levels / blocks")
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        it.build_codectest(targets, out)
    suites = []
    state = {}
    for target in targets:
        print(f"==== {target}")
        runner = it.Runner(target, out, work_name="bc")
        if want("block"):
            s = it.Suite("block", target)
            suite_block(runner, s, random.Random(a.seed), a.quick)
            suites.append(s)
        if want("image") or want("mesa") or (want("ref") and target == targets[0]):
            s = it.Suite("image", target)
            suite_image(runner, s, random.Random(a.seed + 1), state, a.quick)
            if want("image"):
                suites.append(s)
        if want("mesa") and not a.no_ext and target == targets[0]:
            s = it.Suite("mesa", "ref")
            suite_mesa(s, random.Random(a.seed + 2), state)
            suites.append(s)
        if want("ref") and not a.no_ext and target == targets[0]:
            s = it.Suite("ref", target)
            suite_ref(runner, s, random.Random(a.seed + 3), state)
            suites.append(s)
        if want("edges"):
            s = it.Suite("edges", target)
            suite_edges(runner, s, random.Random(a.seed + 4))
            suites.append(s)
        if want("perf"):
            s = it.Suite("perf", target)
            run_perf(s, it.Runner(target, out, work_name="bcp1", extra=["--threads", "1"]),
                     it.Runner(target, out, work_name="bcpn"), random.Random(a.seed + 5))
            suites.append(s)
    if want("determinism"):
        s = it.Suite("determinism", "+".join(targets))
        run_determinism(s, targets, out, random.Random(a.seed + 6))
        suites.append(s)
    ok = it.report(suites, "bc_check")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
