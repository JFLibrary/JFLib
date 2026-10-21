#!/usr/bin/env python3
"""WebP decoder checks (ASMLIB/src/img/webp.s, webp_vp8l.s, webp_vp8.s).

Every decoded image is compared byte for byte with libwebp 1.3 (WebPDecodeRGBA through
ctypes in WSL: libwebp.so.7), the reference of the plan.  Test files come from libwebp
itself (WebPEncode with explicit WebPConfig settings ffmpeg hides: segments, filter type /
sharpness, partitions, alpha compression / filtering / quality, near-lossless, exact,
sharp YUV, ...), from ffmpeg (libwebp / libwebp_anim with presets, qualities,
compression levels, yuv420p / yuva420p / bgra), and from Python writers (containers,
metadata, animation layouts, error cases).  Inputs are generated (numpy patterns, ffmpeg
test sources); nothing is downloaded.

Suites (--only):
  build     vp8l_build_table (op 121) against a model of libwebp's BuildHuffmanTable
  vp8l      libwebp lossless encodes (methods, qualities, exact, near-lossless, palettes)
  ffll      ffmpeg lossless encodes (-compression_level 0..6, -quality)
  vp8       libwebp lossy encodes (segments, filters, sharpness, partitions, sns, ...)
  fflossy   ffmpeg lossy encodes (presets, qualities, yuv420p / yuva420p)
  alpha     ALPH chunks (raw / VP8L, filters, level reduction) + op 123 vs a model
  container VP8X metadata (ICCP / EXIF / XMP), chunk layouts, padding, probe, errors
  anim      animations: first frame on the canvas (ffmpeg libwebp_anim, Python layouts)
  fuzz      mutated files (truncation, bit flips, structural mutants): no crash / hang;
            where libwebp still decodes a mutant the pixels must match
  perf      12 MP lossy / lossless decode times (op 120; limits 300 / 400 ms)

    python tools/webp_check.py --target both --out build/cw
"""

import hashlib
import os
import pickle
import random
import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402

C = it.C
OP_WTIME, OP_WBUILD, OP_WVP8L, OP_WALPHA, OP_WVP8, OP_WPARSE = 120, 121, 122, 123, 124, 125

# ================================================================== libwebp in WSL (ctypes)

WSL_HELPER = r'''
import ctypes, sys, pickle, struct
L = ctypes.CDLL('libwebp.so.7')
CFG = ['lossless','quality','method','image_hint','target_size','target_PSNR','segments',
       'sns_strength','filter_strength','filter_sharpness','filter_type','autofilter',
       'alpha_compression','alpha_filtering','alpha_quality','pass','show_compressed',
       'preprocessing','partitions','partition_limit','emulate_jpeg_size','thread_level',
       'low_memory','near_lossless','exact','use_delta_palette','use_sharp_yuv','qmin','qmax']
FLT = {'quality', 'target_PSNR'}
class Pic(ctypes.Structure):
    _fields_ = [('use_argb', ctypes.c_int), ('colorspace', ctypes.c_int), ('width', ctypes.c_int),
                ('height', ctypes.c_int), ('y', ctypes.c_void_p), ('u', ctypes.c_void_p),
                ('v', ctypes.c_void_p), ('y_stride', ctypes.c_int), ('uv_stride', ctypes.c_int),
                ('a', ctypes.c_void_p), ('a_stride', ctypes.c_int), ('pad1', ctypes.c_uint32 * 2),
                ('argb', ctypes.c_void_p), ('argb_stride', ctypes.c_int), ('pad2', ctypes.c_uint32 * 3),
                ('writer', ctypes.c_void_p), ('custom_ptr', ctypes.c_void_p),
                ('extra_info_type', ctypes.c_int), ('extra_info', ctypes.c_void_p),
                ('stats', ctypes.c_void_p), ('error_code', ctypes.c_int),
                ('progress_hook', ctypes.c_void_p), ('user_data', ctypes.c_void_p),
                ('pad3', ctypes.c_uint32 * 3), ('pad4', ctypes.c_void_p), ('pad5', ctypes.c_void_p),
                ('pad6', ctypes.c_uint32 * 8), ('memory_', ctypes.c_void_p),
                ('memory_argb_', ctypes.c_void_p), ('pad7', ctypes.c_void_p * 2)]
class MemW(ctypes.Structure):
    _fields_ = [('mem', ctypes.c_void_p), ('size', ctypes.c_size_t), ('max_size', ctypes.c_size_t),
                ('pad', ctypes.c_uint32 * 1)]
WRITE = ctypes.cast(L.WebPMemoryWrite, ctypes.c_void_p).value
L.WebPDecodeRGBA.restype = ctypes.c_void_p
def encode(w, h, rgba, cfg, preset):
    cbuf = ctypes.create_string_buffer(512)
    if not L.WebPConfigInitInternal(cbuf, preset, ctypes.c_float(float(cfg.get('quality', 75))), 0x0200):
        return None, 'cfginit'
    if cfg.get('lossless_preset') is not None:
        L.WebPConfigLosslessPreset(cbuf, cfg['lossless_preset'])
    for i, k in enumerate(CFG):
        if k in cfg:
            struct.pack_into('<f' if k in FLT else '<i', cbuf, 4 * i, cfg[k])
    if not L.WebPValidateConfig(cbuf):
        return None, 'badcfg'
    pic = Pic()
    if not L.WebPPictureInitInternal(ctypes.byref(pic), 0x0200):
        return None, 'picinit'
    pic.use_argb = 1 if cfg.get('lossless', 0) else 0
    pic.width, pic.height = w, h
    buf = ctypes.create_string_buffer(bytes(rgba), len(rgba))
    if not L.WebPPictureImportRGBA(ctypes.byref(pic), buf, 4 * w):
        return None, 'import'
    mw = MemW()
    L.WebPMemoryWriterInit(ctypes.byref(mw))
    pic.writer = WRITE
    pic.custom_ptr = ctypes.cast(ctypes.byref(mw), ctypes.c_void_p).value
    ok = L.WebPEncode(cbuf, ctypes.byref(pic))
    err = pic.error_code
    L.WebPPictureFree(ctypes.byref(pic))
    out = ctypes.string_at(mw.mem, mw.size) if ok else None
    L.WebPMemoryWriterClear(ctypes.byref(mw))
    return out, (None if ok else 'enc %d' % err)
def decode(data):
    w = ctypes.c_int(); h = ctypes.c_int()
    p = L.WebPDecodeRGBA(bytes(data), len(data), ctypes.byref(w), ctypes.byref(h))
    if not p:
        return None
    out = ctypes.string_at(p, w.value * h.value * 4)
    L.WebPFree(ctypes.c_void_p(p))
    return (w.value, h.value, out)
jobs = pickle.loads(sys.stdin.buffer.read())
res = []
for j in jobs:
    if j[0] == 'enc':
        res.append(encode(j[1], j[2], j[3], j[4], j[5] if len(j) > 5 else 0))
    else:
        res.append(decode(j[1]))
sys.stdout.buffer.write(pickle.dumps(res, protocol=4))
'''


class Lib:
    """Batched libwebp access (one WSL process per run() call), results cached by content."""

    def __init__(self, cache_dir):
        self.cache_dir = Path(cache_dir)
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        self.mem = {}

    def _key(self, job):
        return hashlib.sha1(pickle.dumps(job, protocol=4)).hexdigest()

    def run(self, jobs):
        out = [None] * len(jobs)
        todo = []
        for i, j in enumerate(jobs):
            k = self._key(j)
            if k in self.mem:
                out[i] = self.mem[k]
                continue
            f = self.cache_dir / (k + ".pkl")
            if f.exists():
                out[i] = self.mem[k] = pickle.loads(f.read_bytes())
                continue
            todo.append((i, k, j))
        for s in range(0, len(todo), 400):
            part = todo[s:s + 400]
            p = it.wsl_python(WSL_HELPER, pickle.dumps([j for _, _, j in part], protocol=4), timeout=3600)
            if p.returncode != 0:
                raise RuntimeError("WSL libwebp helper failed: " + p.stderr.decode(errors="replace")[-400:])
            res = pickle.loads(p.stdout)
            for (i, k, _), r in zip(part, res):
                out[i] = self.mem[k] = r
                (self.cache_dir / (k + ".pkl")).write_bytes(pickle.dumps(r, protocol=4))
        return out

    def encode(self, specs):
        """specs: list of (rgba (h, w, 4) uint8, cfg dict[, preset]) -> list of bytes / None"""
        jobs = []
        for sp in specs:
            px, cfg = sp[0], sp[1]
            preset = sp[2] if len(sp) > 2 else 0
            h, w = px.shape[:2]
            jobs.append(("enc", w, h, np.ascontiguousarray(px, np.uint8).tobytes(), dict(cfg), preset))
        return [r[0] for r in self.run(jobs)]

    def decode(self, files):
        """-> list of (h, w, 4) BGRA arrays or None"""
        res = self.run([("dec", bytes(f)) for f in files])
        out = []
        for r in res:
            if r is None:
                out.append(None)
                continue
            w, h, b = r
            px = np.frombuffer(b, np.uint8).reshape(h, w, 4)
            out.append(px[..., [2, 1, 0, 3]].copy())
        return out


# ================================================================== test images

def ffmpeg_src(src, w, h):
    """RGBA frame of an ffmpeg lavfi test source (rendered at an even size >= 16, cropped)."""
    rw, rh = max(16, (w + 1) & ~1), max(16, (h + 1) & ~1)
    p = subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", f"{src}=size={rw}x{rh}:rate=1",
                        "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"],
                       capture_output=True, timeout=300)
    if p.returncode != 0 or len(p.stdout) < rw * rh * 4:
        raise RuntimeError(f"ffmpeg test source {src} {rw}x{rh} failed: {p.stderr[-200:]!r}")
    return np.frombuffer(p.stdout, np.uint8, count=rw * rh * 4).reshape(rh, rw, 4)[:h, :w].copy()


def add_alpha(px, kind, rng):
    px = px.copy()
    h, w = px.shape[:2]
    if kind == "opaque":
        px[..., 3] = 255
    elif kind == "binary":
        px[..., 3] = np.where(rng.integers(0, 2, (h, w)) == 1, 255, 0)
    elif kind == "random":
        px[..., 3] = rng.integers(0, 256, (h, w))
    elif kind == "grad":
        px[..., 3] = (np.add.outer(np.arange(h) * 255 // max(1, h - 1), np.arange(w) * 0) // 2 +
                      np.add.outer(np.arange(h) * 0, np.arange(w) * 255 // max(1, w - 1)) // 2)
    elif kind == "blobs":
        yy, xx = np.mgrid[0:h, 0:w]
        px[..., 3] = np.where(((yy - h / 2) ** 2 + (xx - w / 3) ** 2) < (min(w, h) / 3) ** 2, 255,
                              np.clip((xx * 7 + yy * 3) % 256, 0, 255))
    return px


def corpus(rng, big=False):
    """list of (name, RGBA (h, w, 4) uint8)"""
    out = []
    sizes = [(1, 1), (1, 7), (7, 1), (2, 2), (3, 5), (16, 16), (17, 13), (33, 65), (65, 33),
             (127, 3), (100, 75)]
    if big:
        sizes += [(256, 256), (641, 479)]
    for (w, h) in sizes:
        out.append((f"noise{w}x{h}", rng.integers(0, 256, (h, w, 4), dtype=np.uint8)))
        g = np.zeros((h, w, 4), np.uint8)
        g[..., 0] = (np.arange(w) * 255 // max(1, w - 1))[None, :]
        g[..., 1] = (np.arange(h) * 255 // max(1, h - 1))[:, None]
        g[..., 2] = ((np.arange(w)[None, :] + np.arange(h)[:, None]) * 3) % 256
        g[..., 3] = 255
        out.append((f"grad{w}x{h}", g))
    for n, (w, h) in [(2, (70, 9)), (3, (31, 17)), (4, (33, 29)), (5, (40, 40)), (16, (37, 21)),
                      (17, (45, 12)), (200, (64, 64)), (256, (50, 50))]:
        pal = rng.integers(0, 256, (n, 4), dtype=np.uint8)
        out.append((f"pal{n}_{w}x{h}", pal[rng.integers(0, n, (h, w))]))
    blk = rng.integers(0, 256, (8, 8, 4), dtype=np.uint8).repeat(8, 0).repeat(8, 1)
    out.append(("blocks64", blk))
    for src, (w, h) in [("testsrc2", (160, 120)), ("mandelbrot", (97, 67)), ("smptehdbars", (128, 72)),
                        ("rgbtestsrc", (64, 48)), ("cellauto", (81, 40)), ("testsrc2", (33, 17))]:
        out.append((f"{src}{w}x{h}", ffmpeg_src(src, w, h)))
    if big:
        for src, (w, h) in [("testsrc2", (720, 480)), ("mandelbrot", (513, 385))]:
            out.append((f"{src}{w}x{h}", ffmpeg_src(src, w, h)))
    return out


def with_alphas(imgs, rng):
    out = []
    kinds = ["opaque", "binary", "random", "grad", "blobs"]
    for i, (name, px) in enumerate(imgs):
        k = kinds[i % len(kinds)]
        out.append((f"{name}-{k}", add_alpha(px, k, rng)))
    return out


# ================================================================== records

def decex_rec(data, exp_bgra, name, flags=C.IMGD_STRAIGHT, meta=None):
    return it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=flags),
                  it.expect_decex(exp_bgra, fmt=C.FMT_ARGB, meta=meta), name)


def ref_recs(lib, files, names, strict_fail=True):
    """op 240 records comparing with libwebp's decode of each file."""
    refs = lib.decode(files)
    recs, skipped = [], 0
    for f, n, r in zip(files, names, refs):
        if f is None:
            skipped += 1
            continue
        if r is None:
            if strict_fail:
                recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(f, flags=C.IMGD_STRAIGHT),
                                   lambda res, out: None if res < 0 else "libwebp rejects, we decode",
                                   n + " (libwebp error)"))
            continue
        recs.append(decex_rec(f, r, n))
    return recs, skipped


# ================================================================== prefix code builder model

def _next_key(key, ln):
    step = 1 << (ln - 1)
    while key & step:
        step >>= 1
    return (key & (step - 1)) + step if step else key


def _next_table_bits(count, ln, root):
    left = 1 << (ln - root)
    while ln < 15:
        left -= count[ln]
        if left <= 0:
            break
        ln += 1
        left <<= 1
    return ln - root


def build_model(lengths, root):
    """libwebp BuildHuffmanTable -> (entries, [(bits, value)])"""
    n = len(lengths)
    count = [0] * 16
    for ln in lengths:
        count[ln] += 1
    if count[0] == n:
        return 0, []
    offset = [0] * 16
    for ln in range(1, 15):
        if count[ln] > (1 << ln):
            return 0, []
        offset[ln + 1] = offset[ln] + count[ln]
    srt = [0] * n
    for s, ln in enumerate(lengths):
        if ln:
            srt[offset[ln]] = s
            offset[ln] += 1
    total = 1 << root
    table = [None] * 70000
    if offset[15] == 1:
        return total, [(0, srt[0])] * total
    low, mask, key, nodes, opn, tsize, sym, tab = -1, total - 1, 0, 1, 1, total, 0, 0
    step = 2
    for ln in range(1, root + 1):
        opn <<= 1
        nodes += opn
        opn -= count[ln]
        if opn < 0:
            return 0, []
        while count[ln] > 0:
            e = tsize
            while True:
                e -= step
                table[key + e] = (ln, srt[sym])
                if e <= 0:
                    break
            sym += 1
            key = _next_key(key, ln)
            count[ln] -= 1
        step <<= 1
    step = 2
    for ln in range(root + 1, 16):
        opn <<= 1
        nodes += opn
        opn -= count[ln]
        if opn < 0:
            return 0, []
        while count[ln] > 0:
            if (key & mask) != low:
                tab += tsize
                tb = _next_table_bits(count, ln, root)
                tsize = 1 << tb
                total += tsize
                low = key & mask
                table[low] = (tb + root, tab - low)
            e = tsize
            while True:
                e -= step
                table[tab + (key >> root) + e] = (ln - root, srt[sym])
                if e <= 0:
                    break
            sym += 1
            key = _next_key(key, ln)
            count[ln] -= 1
        step <<= 1
    if nodes != 2 * offset[15] - 1:
        return 0, []
    return total, table[:total]


def suite_build(runner, target, a):
    rng = random.Random(a.seed)
    s = it.Suite("build", target)

    def rand_code(n, maxlen):
        lens = [0] * n
        k = rng.randint(1, min(n, 80))
        syms = rng.sample(range(n), k)
        ls = [rng.randint(1, maxlen)] if k == 1 else [1, 1]
        while len(ls) < k:
            cands = [i for i, ln in enumerate(ls) if ln < maxlen]
            i = rng.choice(cands)
            ln = ls.pop(i) + 1
            ls += [ln, ln]
        for sy, ln in zip(syms, ls):
            lens[sy] = ln
        return lens

    recs = []
    for t in range(600):
        n = rng.choice([19, 40, 256, 280, 281, 296, 2328])
        root = 7 if n == 19 else 8
        lens = rand_code(n, 7 if n == 19 else 15)
        if t % 7 == 0:                            # possibly invalid (incomplete / over-full)
            lens[rng.randrange(n)] = rng.randint(0, 15 if n != 19 else 7)
        if t % 50 == 0:
            lens = [0] * n
        tot, tab = build_model(lens, root)
        exp = b"".join(struct.pack("<I", (v << 16) | b) for b, v in tab)

        def chk(res, out, tot=tot, exp=exp):
            if res != tot:
                return f"entries {res} != {tot}"
            if bytes(out) != exp:
                return "table differs"
            return None
        recs.append(it.Rec(OP_WBUILD, 0, struct.pack("<II", n, root) + bytes(lens), chk, f"n{n} #{t}"))
    it.run_suite(runner, s, recs)
    return s


# ================================================================== lossless suites

LL_CFGS = [
    {"lossless": 1, "method": 0, "quality": 0},
    {"lossless": 1, "method": 0, "quality": 100, "exact": 1},
    {"lossless": 1, "method": 1, "quality": 50},
    {"lossless": 1, "method": 2, "quality": 75, "exact": 1},
    {"lossless": 1, "method": 3, "quality": 25},
    {"lossless": 1, "method": 4, "quality": 75},
    {"lossless": 1, "method": 4, "quality": 100, "exact": 1},
    {"lossless": 1, "method": 5, "quality": 90},
    {"lossless": 1, "method": 6, "quality": 100, "exact": 1},
    {"lossless": 1, "method": 6, "quality": 0},
    {"lossless": 1, "method": 4, "quality": 75, "near_lossless": 60},
    {"lossless": 1, "method": 5, "quality": 100, "near_lossless": 20},
    {"lossless": 1, "method": 3, "quality": 75, "image_hint": 1},
    {"lossless": 1, "method": 4, "quality": 75, "image_hint": 2},
    {"lossless": 1, "method": 4, "quality": 75, "image_hint": 3, "exact": 1},
]


def suite_vp8l(runner, target, a, lib, cache):
    if "vp8l" not in cache:
        rng = np.random.default_rng(a.seed)
        imgs = with_alphas(corpus(rng, big=not a.quick), rng)
        specs, names = [], []
        for i, (name, px) in enumerate(imgs):
            cfgs = LL_CFGS if px.shape[0] * px.shape[1] <= 20000 else LL_CFGS[::4]
            for k, cfg in enumerate(cfgs):
                if (i + k) % 2 and px.shape[0] * px.shape[1] > 2000:
                    continue
                specs.append((px, cfg))
                names.append(f"{name} {cfg}")
        files = lib.encode(specs)
        cache["vp8l"] = ref_recs(lib, files, names)
    recs, skipped = cache["vp8l"]
    s = it.Suite("vp8l", target)
    it.run_suite(runner, s, recs)
    if skipped:
        s.notes.append(f"{skipped} encodes failed")
    return s


def ffmpeg_file(px, args, work, in_pix="rgba"):
    work.mkdir(parents=True, exist_ok=True)
    h = hashlib.sha1(px.tobytes() + repr(args).encode() + in_pix.encode()).hexdigest()[:16]
    out = work / f"ff_{h}.webp"
    if not out.exists():
        tmp = work / f"ff_{h}.tmp.webp"
        if not it.ffmpeg_encode(px, tmp, list(args), in_pix=in_pix):
            return None
        tmp.replace(out)
    return out.read_bytes()


def suite_ffll(runner, target, a, lib, cache):
    if "ffll" not in cache:
        rng = np.random.default_rng(a.seed + 1)
        imgs = with_alphas(corpus(rng), rng)
        work = Path(a.outdir) / "webpcheck" / "ff"
        files, names = [], []
        for i, (name, px) in enumerate(imgs):
            for lvl in range(7):
                q = (0, 50, 100)[(i + lvl) % 3]
                args = ["-c:v", "libwebp", "-lossless", "1", "-compression_level", str(lvl),
                        "-quality", str(q), "-pix_fmt", "bgra"]
                files.append(ffmpeg_file(px, args, work))
                names.append(f"{name} ff-ll lvl{lvl} q{q}")
        cache["ffll"] = ref_recs(lib, files, names)
    recs, skipped = cache["ffll"]
    s = it.Suite("ffll", target)
    it.run_suite(runner, s, recs)
    if skipped:
        s.notes.append(f"{skipped} ffmpeg encodes failed")
    return s


# ================================================================== lossy suites

LY_CFGS = [
    {"quality": 0},
    {"quality": 5, "method": 0},
    {"quality": 25, "method": 2},
    {"quality": 50, "method": 4, "segments": 1},
    {"quality": 75},
    {"quality": 90, "method": 6},
    {"quality": 100, "method": 6, "segments": 4},
    {"quality": 100, "method": 3, "sns_strength": 0, "filter_strength": 0},
    {"quality": 75, "filter_strength": 0},
    {"quality": 75, "filter_type": 0, "filter_strength": 60},
    {"quality": 30, "filter_type": 0, "filter_strength": 100, "filter_sharpness": 5},
    {"quality": 60, "filter_type": 1, "filter_strength": 100, "filter_sharpness": 7},
    {"quality": 40, "filter_type": 1, "filter_strength": 30, "filter_sharpness": 3},
    {"quality": 20, "filter_type": 1, "filter_strength": 80, "filter_sharpness": 1, "segments": 2},
    {"quality": 75, "partitions": 3},
    {"quality": 80, "partitions": 2, "segments": 3},
    {"quality": 70, "partitions": 1, "method": 1},
    {"quality": 70, "sns_strength": 100, "segments": 4},
    {"quality": 75, "autofilter": 1},
    {"quality": 75, "use_sharp_yuv": 1},
    {"quality": 50, "preprocessing": 2},
    {"quality": 85, "pass": 3, "method": 5},
    {"quality": 95, "method": 2, "segments": 2, "filter_strength": 10},
]
PRESETS = [1, 2, 3, 4, 5]


def lossy_specs(imgs, with_presets=True):
    specs, names = [], []
    for i, (name, px) in enumerate(imgs):
        big = px.shape[0] * px.shape[1] > 20000
        for k, cfg in enumerate(LY_CFGS):
            if big and (i + k) % 5:
                continue
            specs.append((px, cfg))
            names.append(f"{name} {cfg}")
        if with_presets:
            for pr in PRESETS:
                q = (10, 35, 60, 85, 99)[(i + pr) % 5]
                specs.append((px, {"quality": q}, pr))
                names.append(f"{name} preset {pr} q{q}")
    return specs, names


def suite_vp8(runner, target, a, lib, cache):
    if "vp8" not in cache:
        rng = np.random.default_rng(a.seed + 2)
        imgs = corpus(rng, big=not a.quick)
        for _, px in imgs:
            px[..., 3] = 255
        specs, names = lossy_specs(imgs)
        files = lib.encode(specs)
        cache["vp8"] = ref_recs(lib, files, names)
    recs, skipped = cache["vp8"]
    s = it.Suite("vp8", target)
    it.run_suite(runner, s, recs)
    if skipped:
        s.notes.append(f"{skipped} encodes failed")
    return s


def suite_fflossy(runner, target, a, lib, cache):
    if "fflossy" not in cache:
        rng = np.random.default_rng(a.seed + 3)
        imgs = with_alphas(corpus(rng), rng)
        work = Path(a.outdir) / "webpcheck" / "ff"
        files, names = [], []
        presets = ["picture", "photo", "drawing", "icon", "text", "default"]
        for i, (name, px) in enumerate(imgs):
            for k, q in enumerate((5, 25, 50, 75, 90, 100)):
                pf = "yuva420p" if (i + k) % 2 else "yuv420p"
                pr = presets[(i + k) % len(presets)]
                args = ["-c:v", "libwebp", "-lossless", "0", "-quality", str(q), "-preset", pr,
                        "-compression_level", str((i + k) % 7), "-pix_fmt", pf]
                files.append(ffmpeg_file(px, args, work))
                names.append(f"{name} ff q{q} {pr} {pf}")
        cache["fflossy"] = ref_recs(lib, files, names)
    recs, skipped = cache["fflossy"]
    s = it.Suite("fflossy", target)
    it.run_suite(runner, s, recs)
    if skipped:
        s.notes.append(f"{skipped} ffmpeg encodes failed")
    return s


# ================================================================== RIFF helpers

def chunks_of(riff):
    """[(fourcc, payload)] of a RIFF WEBP file (top level)."""
    out, p = [], 12
    end = min(len(riff), 8 + struct.unpack_from("<I", riff, 4)[0])
    while p + 8 <= end:
        tag = riff[p:p + 4]
        n = struct.unpack_from("<I", riff, p + 4)[0]
        out.append((tag, riff[p + 8:p + 8 + n]))
        p += 8 + n + (n & 1)
    return out


def chunk_of(riff, tag):
    for t, d in chunks_of(riff):
        if t == tag:
            return d
    return None


def vp8x(w, h, flags):
    return it.riff_chunk(b"VP8X", struct.pack("<I", flags) + struct.pack("<I", w - 1)[:3] +
                         struct.pack("<I", h - 1)[:3])


def webp(chunks):
    return it.riff(b"WEBP", chunks)


VX_ICC, VX_ALPHA, VX_EXIF, VX_XMP, VX_ANIM = 0x20, 0x10, 0x08, 0x04, 0x02


# ================================================================== alpha filters (model)

def alpha_filter(a, f):
    """libwebp forward filters (filters.c) of an (h, w) uint8 plane -> filtered bytes."""
    if f == 0:
        return a.copy()
    a = a.astype(np.int32)
    h, w = a.shape
    out = np.zeros_like(a)
    for y in range(h):
        for x in range(w):
            if y == 0:
                pred = 0 if x == 0 else a[y, x - 1]
            elif f == 1:
                pred = a[y - 1, 0] if x == 0 else a[y, x - 1]
            elif f == 2:
                pred = a[y - 1, x]
            else:
                if x == 0:
                    pred = a[y - 1, 0]
                else:
                    g = a[y, x - 1] + a[y - 1, x] - a[y - 1, x - 1]
                    pred = min(255, max(0, g))
            out[y, x] = (a[y, x] - pred) & 255
    return out.astype(np.uint8)


def suite_alpha(runner, target, a, lib, cache):
    if "alpha" not in cache:
        rng = np.random.default_rng(a.seed + 4)
        imgs = [(n, px) for n, px in with_alphas(corpus(rng), rng)
                if not n.endswith("-opaque")]
        specs, names = [], []
        acfgs = [dict(alpha_compression=c, alpha_filtering=f, alpha_quality=q)
                 for c in (0, 1) for f in (0, 1, 2) for q in (100, 60, 0)]
        for i, (name, px) in enumerate(imgs):
            for k, ac in enumerate(acfgs):
                if (i + k) % 3:
                    continue
                cfg = dict(quality=(20, 50, 80, 95)[(i + k) % 4], **ac)
                if (i + k) % 5 == 0:
                    cfg["exact"] = 1
                specs.append((px, cfg))
                names.append(f"{name} {cfg}")
        files = lib.encode(specs)
        recs, skipped = ref_recs(lib, files, names)
        # hand-made ALPH chunks: every filter, raw and VP8L-compressed, level-reduction flag
        base = [(n, px) for n, px in imgs if px.shape[0] * px.shape[1] <= 5000][:12]
        made = []
        for i, (name, px) in enumerate(base):
            h, w = px.shape[:2]
            opaque = px.copy()
            opaque[..., 3] = 255
            vp8_file = lib.encode([(opaque, {"quality": 70})])[0]
            vp8 = chunk_of(vp8_file, b"VP8 ")
            al = px[..., 3]
            for f in range(4):
                filt = alpha_filter(al, f)
                pre = (i + f) & 1
                hdr = 0 | (f << 2) | (pre << 4)
                raw = bytes([hdr]) + filt.tobytes()
                made.append((webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", raw),
                                   it.riff_chunk(b"VP8 ", vp8)]), f"{name} ALPH raw f{f} pre{pre}"))
                g = np.zeros((h, w, 4), np.uint8)
                g[..., 1] = filt
                g[..., 3] = 255
                ll = lib.encode([(g, {"lossless": 1, "method": (i + f) % 7, "quality": 50,
                                      "exact": 1})])[0]
                vl = chunk_of(ll, b"VP8L")
                comp = bytes([1 | (f << 2) | (pre << 4)]) + vl[5:]
                made.append((webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", comp),
                                   it.riff_chunk(b"VP8 ", vp8)]), f"{name} ALPH vp8l f{f} pre{pre}"))
        r2, _ = ref_recs(lib, [m[0] for m in made], [m[1] for m in made])
        recs += r2
        # op 123: webp_alpha against the model (plane bytes)
        for i in range(60):
            w, h = int(rng.integers(1, 40)), int(rng.integers(1, 30))
            al = rng.integers(0, 256, (h, w), dtype=np.uint8)
            if i % 3 == 0:
                al = np.clip(np.add.outer(np.arange(h) * 9, np.arange(w) * 5), 0, 255).astype(np.uint8)
            f = i % 4
            filt = alpha_filter(al, f)
            hdr = (f << 2) | ((i >> 2) & 1) << 4
            data = struct.pack("<II", w, h) + bytes([hdr]) + filt.tobytes()
            recs.append(it.Rec(OP_WALPHA, 0, data, it.expect_bytes(al.tobytes()), f"op123 {w}x{h} f{f}"))
        bad = [(bytes([2]) + b"\0" * 20, C.ERR_HDR, "method 2"), (bytes([3]) + b"\0" * 20, C.ERR_HDR, "method 3"),
               (bytes([0x40]) + b"\0" * 20, C.ERR_HDR, "reserved bits"),
               (bytes([0x20]) + b"\0" * 20, C.ERR_HDR, "pre-processing 2"),
               (bytes([0]), C.ERR_HDR, "header only"), (b"", C.ERR_HDR, "empty"),
               (bytes([0]) + b"\0" * 19, C.ERR_TRUNC, "raw short"),
               (bytes([1]) + b"\xff" * 3, None, "vp8l garbage")]
        for payload, err, n in bad:
            data = struct.pack("<II", 5, 4) + payload
            chk = it.expect_err(err) if err is not None else (lambda res, out: None if res < 0 else "accepted")
            recs.append(it.Rec(OP_WALPHA, 0, data, chk, "op123 " + n))
        cache["alpha"] = (recs, skipped)
    recs, skipped = cache["alpha"]
    s = it.Suite("alpha", target)
    it.run_suite(runner, s, recs)
    if skipped:
        s.notes.append(f"{skipped} encodes failed")
    return s


# ================================================================== container / metadata / probe

def meta_check(fields):
    def chk(res, out):
        if res < 0:
            return f"error {it.err_name(res)}"
        d = it.parse_decex(out)
        for k, v in fields.items():
            if k in ("icc", "exif", "xmp"):
                if d[k] != v:
                    return f"{k} blob differs ({len(d[k])} bytes vs {len(v)})"
            elif d["meta"][k] != v:
                return f"meta {k} = {d['meta'][k]}, expected {v}"
        return None
    return chk


def probe_check(fmt, fields):
    def chk(res, out):
        if res != fmt:
            return f"probe {res} != {fmt}"
        m = it.parse_meta_rec(out)
        for k, v in fields.items():
            if m[k] != v:
                return f"probe {k} = {m[k]}, expected {v}"
        return None
    return chk


def both(*checks):
    def chk(res, out):
        for c in checks:
            m = c(res, out)
            if m:
                return m
        return None
    return chk


def suite_container(runner, target, a, lib, cache):
    if "container" not in cache:
        rng = np.random.default_rng(a.seed + 5)
        px = add_alpha(ffmpeg_src("testsrc2", 37, 23), "random", rng)
        opq = px.copy()
        opq[..., 3] = 255
        ll_file, ly_file, lya_file = lib.encode([(px, {"lossless": 1, "quality": 60, "exact": 1}),
                                                 (opq, {"quality": 60}),
                                                 (px, {"quality": 60, "alpha_compression": 1})])
        vl = chunk_of(ll_file, b"VP8L")
        v8 = chunk_of(ly_file, b"VP8 ")
        alph = chunk_of(lya_file, b"ALPH")
        v8a = chunk_of(lya_file, b"VP8 ")
        w, h = 37, 23
        icc = bytes(range(256)) * 3 + b"icc!"
        exif = it.exif_blob(orient=6, xres=(300, 1), yres=(300, 1), unit=2)
        xmp = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF/></x:xmpmeta>"
        allf = C.IMGD_STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP | C.IMGD_NO_ORIENT
        files = []           # (bytes, name, check or None = compare with libwebp, flags)

        def add(data, name, chk=None, flags=C.IMGD_STRAIGHT, ref=True):
            files.append((data, name, chk, flags, ref))
        add(webp([it.riff_chunk(b"VP8L", vl)]), "simple VP8L",
            meta_check(dict(w=w, h=h, bits=8, model=C.IMM_RGB, channels=4,
                            flags=C.IMF_ALPHA | C.IMF_LOSSLESS, frames=1)))
        add(webp([it.riff_chunk(b"VP8 ", v8)]), "simple VP8",
            meta_check(dict(w=w, h=h, bits=8, model=C.IMM_YCC, channels=3, flags=0, frames=1)))
        add(webp([vp8x(w, h, 0), it.riff_chunk(b"VP8L", vl)]), "VP8X VP8L")
        add(webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", alph), it.riff_chunk(b"VP8 ", v8a)]),
            "VP8X ALPH VP8", meta_check(dict(channels=4, flags=C.IMF_ALPHA)))
        add(webp([vp8x(w, h, 0), it.riff_chunk(b"ALPH", alph), it.riff_chunk(b"VP8 ", v8a)]),
            "ALPH without the VP8X alpha flag")
        add(webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", b"\x00" + b"\x10" * (w * h)),
                  it.riff_chunk(b"ALPH", alph), it.riff_chunk(b"VP8 ", v8a)]), "two ALPH (last wins)")
        add(webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", alph), it.riff_chunk(b"VP8L", vl)]),
            "ALPH ignored for VP8L")
        add(webp([vp8x(w, h, 0), it.riff_chunk(b"ABCD", b"xyz"), it.riff_chunk(b"VP8L", vl),
                  it.riff_chunk(b"ZZZZ", b"1")]), "unknown chunks (odd sizes)")
        full = webp([vp8x(w, h, VX_ICC | VX_EXIF | VX_XMP), it.riff_chunk(b"ICCP", icc),
                     it.riff_chunk(b"VP8 ", v8), it.riff_chunk(b"EXIF", exif), it.riff_chunk(b"XMP ", xmp)])
        add(full, "metadata blobs", meta_check(dict(icc=icc, exif=exif, xmp=xmp, orient=6)), allf)
        add(full, "metadata not requested", meta_check(dict(icclen=0, exiflen=0, xmplen=0, orient=6)),
            C.IMGD_STRAIGHT | C.IMGD_NO_ORIENT)
        noflags = webp([vp8x(w, h, 0), it.riff_chunk(b"ICCP", icc), it.riff_chunk(b"VP8 ", v8),
                        it.riff_chunk(b"EXIF", exif), it.riff_chunk(b"XMP ", xmp)])
        add(noflags, "metadata without VP8X flags", meta_check(dict(icclen=0, exiflen=0, xmplen=0, orient=1)),
            allf)
        pre = webp([vp8x(w, h, VX_EXIF), it.riff_chunk(b"VP8L", vl),
                    it.riff_chunk(b"EXIF", b"Exif\0\0" + exif)])
        add(pre, "EXIF with Exif00 prefix", meta_check(dict(exif=exif, orient=6)), allf)
        twoicc = webp([vp8x(w, h, VX_ICC), it.riff_chunk(b"ICCP", icc), it.riff_chunk(b"ICCP", b"second"),
                       it.riff_chunk(b"VP8L", vl)])
        add(twoicc, "first ICCP wins", meta_check(dict(icc=icc)), allf)
        broken_tail = webp([vp8x(w, h, VX_XMP), it.riff_chunk(b"VP8L", vl)]) + b"XMP \xff\xff\xff\x7f"
        broken_tail = broken_tail[:4] + struct.pack("<I", len(broken_tail) - 8) + broken_tail[8:]
        add(broken_tail, "broken chunk after the image", meta_check(dict(xmplen=0)), allf)
        trailing = webp([it.riff_chunk(b"VP8L", vl)]) + b"garbage after the RIFF"
        add(trailing, "bytes after the RIFF")
        # orientation applied by the dispatcher (IMGD_NO_ORIENT off)
        ref = lib.decode([full])[0]
        add(full, "EXIF orientation 6 applied", it.expect_decex(it.orient(ref, 6)), C.IMGD_STRAIGHT, False)
        # errors
        simple = webp([it.riff_chunk(b"VP8L", vl)])
        errs = [
            (simple[:11], C.ERR_UNSUPPORTED, "11 bytes"),
            (simple[:4] + struct.pack("<I", len(simple)) + simple[8:], C.ERR_TRUNC, "RIFF size > data"),
            (simple[:4] + struct.pack("<I", 11) + simple[8:], C.ERR_HDR, "RIFF size 11"),
            (simple[:4] + struct.pack("<I", 0xFFFFFFFF) + simple[8:], C.ERR_HDR, "RIFF size 4G"),
            (simple[:40], C.ERR_TRUNC, "truncated VP8L"),
            (webp([it.riff_chunk(b"VP8X", b"\0" * 9), it.riff_chunk(b"VP8L", vl)]), C.ERR_HDR, "VP8X size 9"),
            (webp([vp8x(w + 1, h, 0), it.riff_chunk(b"VP8L", vl)]), C.ERR_HDR, "canvas != image"),
            (webp([vp8x(w, h, 0), it.riff_chunk(b"ICCP", icc)]), C.ERR_TRUNC, "no image chunk"),
            (webp([it.riff_chunk(b"ABCD", b"1234"), it.riff_chunk(b"VP8L", vl)]), C.ERR_HDR, "unknown first chunk"),
            (webp([vp8x(w, h, 0), b"ABCD" + struct.pack("<I", 0x7FFFFFF0) + b"xx", it.riff_chunk(b"VP8L", vl)]),
             C.ERR_HDR, "huge chunk size"),
            (webp([vp8x(0x1000000, 0x1000000, 0), it.riff_chunk(b"VP8L", vl)]), C.ERR_HDR, "canvas 2^48"),
            (webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", b"\x07zz"), it.riff_chunk(b"VP8 ", v8)]),
             C.ERR_HDR, "bad ALPH header"),
            (webp([vp8x(w, h, VX_ALPHA), it.riff_chunk(b"ALPH", b""), it.riff_chunk(b"VP8 ", v8)]),
             C.ERR_HDR, "empty ALPH"),
            (webp([it.riff_chunk(b"VP8 ", bytes([v8[0] | 1]) + v8[1:])]), C.ERR_HDR, "inter frame"),
            (webp([it.riff_chunk(b"VP8 ", v8[:3] + b"\x9d\x01\x2b" + v8[6:])]), C.ERR_HDR, "bad start code"),
            (webp([it.riff_chunk(b"VP8 ", v8[:6] + b"\0\0" + v8[8:])]), C.ERR_HDR, "VP8 width 0"),
            (webp([it.riff_chunk(b"VP8L", b"\x2e" + vl[1:])]), C.ERR_HDR, "VP8L signature"),
            (webp([it.riff_chunk(b"VP8L", vl[:4] + bytes([vl[4] | 0x20]) + vl[5:])]), C.ERR_HDR, "VP8L version"),
        ]
        big = bytearray(v8)
        big[0:3] = struct.pack("<I", (struct.unpack("<I", bytes(v8[0:3]) + b"\0")[0] & 0x1F) | (len(v8) << 5))[:3]
        errs.append((webp([it.riff_chunk(b"VP8 ", bytes(big))]), C.ERR_HDR, "first partition >= chunk"))
        recs = []
        cmp_files = [f for f in files if f[2] is None or f[4]]
        refs = lib.decode([f[0] for f in cmp_files])
        for (data, name, chk, flags, ref), r in zip(cmp_files, refs):
            checks = []
            if r is None:
                checks.append(lambda res, out: "libwebp rejects this file")
            else:
                checks.append(it.expect_decex(it.orient(r, 1) if flags & C.IMGD_NO_ORIENT or chk is None else r,
                                              fmt=C.FMT_ARGB))
            if chk is not None:
                checks.append(chk)
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=flags), both(*checks), name))
        for data, name, chk, flags, ref in files:
            if not (chk is None or ref):
                recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=flags), chk, name))
        for data, err, name in errs:
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_STRAIGHT),
                               it.expect_err(err), "error: " + name))
        # probe, IMGD_PROBE, frames
        recs.append(it.Rec(it.OP_PROBE, 0, full, probe_check(C.IMGF_WEBP, dict(
            w=w, h=h, bits=8, model=C.IMM_YCC, channels=3, orient=6, frames=1, icclen=0)), "probe VP8X"))
        recs.append(it.Rec(it.OP_PROBE, 0, simple, probe_check(C.IMGF_WEBP, dict(
            w=w, h=h, model=C.IMM_RGB, channels=4, flags=C.IMF_ALPHA | C.IMF_LOSSLESS)), "probe VP8L"))
        recs.append(it.Rec(it.OP_PROBE, 0, simple[:30], probe_check(C.IMGF_WEBP, dict(w=0)), "probe truncated"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(full, flags=C.IMGD_PROBE | allf),
                           meta_check(dict(w=w, h=h, icc=icc, exif=exif, xmp=xmp)), "IMGD_PROBE"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(simple, frame=1), it.expect_err(C.ERR_LIMIT),
                           "frame 1 of a still image"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(simple, maxpx=w * h - 1), it.expect_err(C.ERR_LIMIT),
                           "ID_MAXPX"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(simple, flags=0),
                           it.expect_decex(it.premul(lib.decode([simple])[0]), fmt=C.FMT_PARGB), "premultiplied"))
        # op 125 webp_parse fields
        def pchk(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            v = struct.unpack_from("<15I6Q", out, 0)
            if v[0] != w or v[1] != h or v[2] != (VX_ICC | VX_EXIF | VX_XMP) or v[3] != 1:
                return f"fields {v}"
            if v[18] != len(icc) or v[19] != len(exif) or v[20] != len(xmp):
                return f"lengths {v[15:]}"
            return None
        recs.append(it.Rec(OP_WPARSE, 0, full, pchk, "webp_parse fields"))
        cache["container"] = recs
    s = it.Suite("container", target)
    it.run_suite(runner, s, cache["container"])
    return s


# ================================================================== animation

def anmf(x, y, w, h, dur, flags, sub):
    return it.riff_chunk(b"ANMF", struct.pack("<I", x // 2)[:3] + struct.pack("<I", y // 2)[:3] +
                         struct.pack("<I", w - 1)[:3] + struct.pack("<I", h - 1)[:3] +
                         struct.pack("<I", dur)[:3] + bytes([flags]) + b"".join(sub))


def first_frame(lib, data):
    """(canvas BGRA with the first frame placed like WebPAnimDecoder, frames, loops, duration)."""
    cs = chunks_of(data)
    vx = cs[0][1]
    cw = 1 + int.from_bytes(vx[4:7], "little")
    ch = 1 + int.from_bytes(vx[7:10], "little")
    frames = [d for t, d in cs if t == b"ANMF"]
    loops = next((struct.unpack_from("<H", d, 4)[0] for t, d in cs if t == b"ANIM"), 0)
    f = frames[0]
    x = 2 * int.from_bytes(f[0:3], "little")
    y = 2 * int.from_bytes(f[3:6], "little")
    dur = int.from_bytes(f[12:15], "little")
    sub, p = [], 16
    while p + 8 <= len(f):
        n = struct.unpack_from("<I", f, p + 4)[0]
        sub.append((f[p:p + 4], f[p + 8:p + 8 + n]))
        p += 8 + n + (n & 1)
    img = next(d for t, d in sub if t in (b"VP8 ", b"VP8L"))
    kind = next(t for t, d in sub if t in (b"VP8 ", b"VP8L"))
    al = next((d for t, d in sub if t == b"ALPH"), None)
    if kind == b"VP8L":
        still = webp([it.riff_chunk(b"VP8L", img)])
    elif al is not None:
        fw = int.from_bytes(img[6:8], "little") & 0x3FFF
        fh = int.from_bytes(img[8:10], "little") & 0x3FFF
        still = webp([vp8x(fw, fh, VX_ALPHA), it.riff_chunk(b"ALPH", al), it.riff_chunk(b"VP8 ", img)])
    else:
        still = webp([it.riff_chunk(b"VP8 ", img)])
    fr = lib.decode([still])[0]
    canvas = np.zeros((ch, cw, 4), np.uint8)
    canvas[y:y + fr.shape[0], x:x + fr.shape[1]] = fr
    return canvas, len(frames), loops, dur


def suite_anim(runner, target, a, lib, cache):
    if "anim" not in cache:
        rng = np.random.default_rng(a.seed + 6)
        work = Path(a.outdir) / "webpcheck" / "anim"
        work.mkdir(parents=True, exist_ok=True)
        recs = []
        # ffmpeg libwebp_anim
        for i, (w, h, nf, lossless, pf, loop) in enumerate([
                (64, 48, 4, 0, "yuv420p", 0), (33, 17, 3, 1, "bgra", 2), (80, 60, 5, 0, "yuva420p", 7),
                (17, 31, 2, 1, "bgra", 0)]):
            frames = []
            for k in range(nf):
                f = ffmpeg_src("testsrc2", w, h)
                f = np.roll(f, 3 * k, axis=1)
                if pf != "yuv420p":
                    f = add_alpha(f, "blobs" if k % 2 else "grad", rng)
                frames.append(f)
            raw = np.concatenate(frames)
            out = work / f"ffanim{i}.webp"
            if not out.exists():
                p = subprocess.run(["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba",
                                    "-s", f"{w}x{h}", "-r", "10", "-i", "-", "-c:v", "libwebp_anim",
                                    "-lossless", str(lossless), "-loop", str(loop), "-pix_fmt", pf,
                                    str(out)], input=raw.tobytes(), capture_output=True, timeout=300)
                if p.returncode != 0:
                    continue
            data = out.read_bytes()
            canvas, n, loops, dur = first_frame(lib, data)
            meta = dict(frames=n, loops=loops, delay=dur, w=w, h=h)
            if n > 1:
                meta["flags"] = (C.IMF_ANIMATED | (C.IMF_LOSSLESS if lossless else 0) |
                                 (C.IMF_ALPHA if (data[20] & VX_ALPHA) else 0))
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_STRAIGHT),
                               both(it.expect_decex(canvas), meta_check(meta)), f"ffmpeg anim {i}"))
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, frame=1), it.expect_err(C.ERR_UNSUPPORTED),
                               f"ffmpeg anim {i} frame 1"))
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, frame=n), it.expect_err(C.ERR_LIMIT),
                               f"ffmpeg anim {i} frame {n}"))
        # Python layouts: offsets, ALPH + VP8, VP8L, several frames
        px = add_alpha(ffmpeg_src("mandelbrot", 40, 30), "random", rng)
        opq = px.copy()
        opq[..., 3] = 255
        ll, ly, lya = lib.encode([(px, {"lossless": 1, "quality": 50, "exact": 1}), (opq, {"quality": 50}),
                                  (px, {"quality": 50, "alpha_compression": 1, "alpha_filtering": 2})])
        vl, v8, alph, v8a = (chunk_of(ll, b"VP8L"), chunk_of(ly, b"VP8 "), chunk_of(lya, b"ALPH"),
                             chunk_of(lya, b"VP8 "))
        anim_hdr = it.riff_chunk(b"ANIM", struct.pack("<IH", 0xFF336699, 5))
        layouts = [
            ("VP8L at (10, 20)", 100, 80, [anmf(10, 20, 40, 30, 70, 0, [it.riff_chunk(b"VP8L", vl)]),
                                          anmf(0, 0, 40, 30, 30, 2, [it.riff_chunk(b"VP8L", vl)])]),
            ("ALPH + VP8 at (0, 2)", 41, 33, [anmf(0, 2, 40, 30, 100, 1,
                                                   [it.riff_chunk(b"ALPH", alph), it.riff_chunk(b"VP8 ", v8a)])]),
            ("VP8 full canvas", 40, 30, [anmf(0, 0, 40, 30, 5, 0, [it.riff_chunk(b"VP8 ", v8)]),
                                         anmf(0, 0, 40, 30, 5, 0, [it.riff_chunk(b"VP8 ", v8)]),
                                         anmf(0, 0, 40, 30, 5, 0, [it.riff_chunk(b"VP8 ", v8)])]),
            ("unknown sub-chunk after the image", 60, 40,
             [anmf(20, 10, 40, 30, 9, 0, [it.riff_chunk(b"VP8L", vl), it.riff_chunk(b"XYZW", b"12345")])]),
        ]
        for name, cw, chh, frames in layouts:
            data = webp([vp8x(cw, chh, VX_ANIM | VX_ALPHA), anim_hdr] + frames)
            canvas, n, loops, dur = first_frame(lib, data)
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_STRAIGHT),
                               both(it.expect_decex(canvas), meta_check(dict(frames=n, loops=5, delay=dur,
                                                                             w=cw, h=chh))), "anim " + name))
        errs = [
            ("ANMF before ANIM", webp([vp8x(40, 30, VX_ANIM), anmf(0, 0, 40, 30, 1, 0, [it.riff_chunk(b"VP8L", vl)]),
                                       anim_hdr])),
            ("frame outside the canvas", webp([vp8x(40, 30, VX_ANIM), anim_hdr,
                                               anmf(2, 0, 40, 30, 1, 0, [it.riff_chunk(b"VP8L", vl)])])),
            ("ALPH + VP8L", webp([vp8x(40, 30, VX_ANIM), anim_hdr,
                                  anmf(0, 0, 40, 30, 1, 0, [it.riff_chunk(b"ALPH", alph), it.riff_chunk(b"VP8L", vl)])])),
            ("bare VP8L in an animation", webp([vp8x(40, 30, VX_ANIM), anim_hdr, it.riff_chunk(b"VP8L", vl)])),
            ("no frames", webp([vp8x(40, 30, VX_ANIM), anim_hdr])),
            ("frame without image", webp([vp8x(40, 30, VX_ANIM), anim_hdr, anmf(0, 0, 40, 30, 1, 0,
                                                                                [it.riff_chunk(b"ALPH", alph)])])),
            ("ANIM too short", webp([vp8x(40, 30, VX_ANIM), it.riff_chunk(b"ANIM", b"1234"),
                                     anmf(0, 0, 40, 30, 1, 0, [it.riff_chunk(b"VP8L", vl)])])),
        ]
        for name, data in errs:
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data), it.expect_err(C.ERR_HDR), "anim error: " + name))
        cache["anim"] = recs
    s = it.Suite("anim", target)
    it.run_suite(runner, s, cache["anim"])
    return s


# ================================================================== fuzz

def suite_fuzz(runner, target, a, lib, cache):
    if "fuzz" not in cache:
        rng = random.Random(a.seed + 7)
        nrng = np.random.default_rng(a.seed + 7)
        seeds = []
        for (w, h) in [(13, 9), (32, 24), (5, 40)]:
            px = add_alpha(ffmpeg_src("testsrc2", w, h), "random", nrng)
            noise = nrng.integers(0, 256, (h, w, 4), dtype=np.uint8)
            pal = nrng.integers(0, 256, (6, 4), dtype=np.uint8)[nrng.integers(0, 6, (h, w))]
            for img in (px, noise, pal):
                seeds += lib.encode([(img, {"lossless": 1, "method": rng.randrange(7), "quality": rng.choice([0, 50, 100])}),
                                     (img, {"quality": rng.choice([10, 60, 95]), "segments": rng.randint(1, 4),
                                            "partitions": rng.randint(0, 3), "alpha_compression": rng.randint(0, 1),
                                            "alpha_filtering": rng.randint(0, 2)})])
        seeds = [s_ for s_ in seeds if s_]
        anim_dir = Path(a.outdir) / "webpcheck" / "anim"
        seeds += [f.read_bytes() for f in sorted(anim_dir.glob("ffanim*.webp"))][:2]
        muts = []
        n = a.fuzz
        for i in range(n):
            base = rng.choice(seeds)
            if i % 7 == 0:
                # structural: chunk sizes / VP8L header / VP8 partitions
                b = bytearray(base)
                k = rng.randrange(5)
                if k == 0:
                    p = rng.choice([4, 16])
                    b[p:p + 4] = struct.pack("<I", rng.choice([0xFFFFFFFF, 0x7FFFFFFF, len(b), len(b) + 1, 0, 11]))
                elif k == 1:
                    cs = [m for m in range(12, len(b) - 8)
                          if bytes(b[m:m + 4]) in (b"VP8 ", b"VP8L", b"ALPH", b"ANMF", b"VP8X", b"ANIM")]
                    if cs:
                        p = rng.choice(cs) + 4
                        b[p:p + 4] = struct.pack("<I", rng.choice([0xFFFFFFF0, 0x80000000, 1, 3, 9, 15, 17]))
                elif k == 2:
                    p = bytes(b).find(b"VP8L")
                    if p > 0 and p + 20 < len(b):
                        for _ in range(rng.randint(1, 4)):
                            q = p + 13 + rng.randrange(8)
                            b[q] ^= 1 << rng.randrange(8)
                elif k == 3:
                    p = bytes(b).find(b"VP8 ")
                    if p > 0 and p + 30 < len(b):
                        q = p + 8 + rng.randrange(22)
                        b[q] = rng.randrange(256)
                else:
                    b = bytearray(it.mutate(rng, bytes(b), keep=12))
                muts.append(bytes(b))
            else:
                muts.append(it.mutate(rng, base, keep=12 if i % 3 else 0))
        refs = lib.decode(muts)
        cache["fuzz"] = (muts, refs)
    muts, refs = cache["fuzz"]
    s = it.Suite("fuzz", target)
    stats = {"both": 0, "we_reject": 0, "lib_reject": 0}

    def mk(ref):
        def chk(res, out):
            if res < 0:
                if ref is not None:
                    stats["we_reject"] += 1
                return None
            d = it.parse_decex(out)
            if ref is None:
                stats["lib_reject"] += 1
                return None
            stats["both"] += 1
            if d["px"] is None or d["px"].shape != ref.shape or it.max_abs(d["px"], ref):
                return "decoded differently from libwebp"
            return None
        return chk
    recs = [it.Rec(it.OP_DECEX, 0, it.decex_input(m, flags=C.IMGD_STRAIGHT, maxpx=1 << 22), mk(r), f"mutant {i}")
            for i, (m, r) in enumerate(zip(muts, refs))]
    it.run_suite(runner, s, recs)
    s.notes.append(f"both decode {stats['both']}, only libwebp {stats['we_reject']}, "
                   f"only ours {stats['lib_reject']} (animations / libwebp-specific acceptance)")
    return s


# ================================================================== performance

def suite_perf(runner, target, a, lib, cache):
    s = it.Suite("perf", target)
    if "perf" not in cache:
        w, h = 4000, 3000
        work = Path(a.outdir) / "webpcheck"
        f1, f2 = work / "perf12_lossy.webp", work / "perf12_lossless.webp"
        if not (f1.exists() and f2.exists()):
            px = ffmpeg_src("testsrc2", w, h)
            m = ffmpeg_src("mandelbrot", w, h)
            px[:, w // 2:] = m[:, w // 2:]
            r = np.random.default_rng(1)
            px[..., :3] = np.clip(px[..., :3].astype(int) + r.integers(-12, 13, (h, w, 3)), 0, 255).astype(np.uint8)
            px[..., 3] = 255
            ly, ll = lib.encode([(px, {"quality": 80}), (px, {"lossless": 1, "quality": 75, "method": 4})])
            f1.write_bytes(ly)
            f2.write_bytes(ll)
        cache["perf"] = [("lossy", f1.read_bytes(), 300), ("lossless", f2.read_bytes(), 400)]
    for name, data, limit in cache["perf"]:
        best = None
        for _ in range(3):
            res, out = runner.one(OP_WTIME, 3 | (C.IMGD_STRAIGHT << 16), data)
            if res != 0:
                s.fail(f"{name}: error {it.err_name(res)}")
                break
            us = struct.unpack_from("<I", out, 0)[0]
            best = us if best is None else min(best, us)
        if best is None:
            continue
        s.notes.append(f"12 MP {name} ({len(data) // 1024} KB): {best / 1000:.1f} ms (limit {limit} ms)")
        s.check(best <= limit * 1000, f"{name} {best / 1000:.1f} ms > {limit} ms")
    return s


# ================================================================== main

SUITES = ["build", "vp8l", "ffll", "vp8", "fflossy", "alpha", "container", "anim", "fuzz", "perf"]


def main():
    ap = it.argparser(__doc__, ",".join(SUITES))
    ap.add_argument("--quick", action="store_true", help="smaller corpus")
    a = ap.parse_args()
    out = it.out_dir(a)
    a.outdir = out
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = [x for x in a.only.split(",") if x] or SUITES
    lib = Lib(out / "webpcheck" / "libwebp_cache")
    cache = {}
    suites = []
    for t in targets:
        runner = it.Runner(t, out, work_name="webpcheck")
        for name in only:
            fn = globals()["suite_" + name]
            if name == "build":
                suites.append(fn(runner, t, a))
            else:
                suites.append(fn(runner, t, a, lib, cache))
    ok = it.report(suites, "webp_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
