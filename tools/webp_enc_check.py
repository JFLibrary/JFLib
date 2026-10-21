#!/usr/bin/env python3
"""webp_enc_check.py -- checker of the WebP encoder (CE: webp_encode, vp8l_enc_argb,
vp8_enc_argb, webp_enc_alpha).

References: libwebp 1.3 through ctypes in WSL (the Lib helper of webp_check.py) decodes
every file we write and encodes the comparison files; our decoder must agree with it.

Suites:
  lossless  img_encode IOF_LOSSLESS round trips (libwebp and our decoder exact) over noise,
            gradients, palettes (1..256 colours, bundling), ffmpeg test sources, photos,
            all alpha kinds (RGB under alpha 0 kept), sizes 1x1 .. 16383x1, efforts 1..9,
            IOF_WEBP_NOEXACT (only transparent pixels may change)
  lossy     VP8 (+ ALPH) files: libwebp decodes them, our decoder is bit-exact with it, alpha
            exact, PSNR in range; qualities 1..100, efforts 1..9, presets 0..5, odd sizes
  alpha     ALPH payloads (op 128) decoded by webp_alpha (op 123) equal the alpha plane
  container simple VP8 / VP8L vs VP8X, ICCP / EXIF (orientation 1) / XMP read back, RIFF
            sizes and padding, capabilities
  errors    16384 pixels -> ERR_DIMS, cancel -> ERR_CANCEL
  size      lossless size vs libwebp -z 6 (target <= 1.10 in total), lossy size / PSNR vs
            libwebp at the same quality (target PSNR >= libwebp - 0.5 dB at <= 1.05 size)
  det       the same file on every run / thread setting; Windows == Linux bytes (--target both)
  perf      12 MP photo: lossless (effort 5) and lossy (q 90) encode times

    python tools/webp_enc_check.py --target both --out build/ce
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
import webp_check as wc  # noqa: E402

C = it.C
OP_WALPHA, OP_WPARSE = 123, 125
OP_WEL, OP_WEV, OP_WEA, OP_WETIME = 126, 127, 128, 129
OP_ENCMETA = 260
SUITES = ["lossless", "lossy", "alpha", "container", "errors", "size", "det", "perf"]
WALL = [("lake", r"C:\Windows\Web\Wallpaper\ThemeC\img29.jpg"), ("hills", r"C:\Windows\Web\Wallpaper\ThemeC\img28.jpg"),
        ("stripes", r"C:\Windows\Web\Wallpaper\ThemeB\img26.jpg"), ("ribbons", r"C:\Windows\Web\Wallpaper\ThemeB\img25.jpg"),
        ("collage", r"C:\Windows\Web\Wallpaper\Spotlight\img50.jpg")]


def imgopt(quality=0, flags=0, effort=0, preset=0, fmt=None):
    return struct.pack("<12IQQ", C.IMGF_WEBP if fmt is None else fmt, quality, flags, 0, 0, 0, 0, 0,
                       effort, preset, 0, 0, 0, 0)


def ffmpeg_bgra(args, w, h):
    p = subprocess.run(["ffmpeg", "-v", "error", *args, "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "bgra", "-"],
                       capture_output=True, timeout=300)
    if p.returncode != 0 or len(p.stdout) < w * h * 4:
        return None
    return np.frombuffer(p.stdout, np.uint8, count=w * h * 4).reshape(h, w, 4).copy()


def photos(out, size=512):
    cache = Path(out) / "webpenc" / f"photos{size}.pkl"
    if cache.exists():
        return pickle.loads(cache.read_bytes())
    res = []
    if it.have_exe("ffmpeg"):
        for n, p in WALL:
            if os.path.exists(p):
                img = ffmpeg_bgra(["-i", p, "-vf", f"scale=960:600,crop={size}:{size}:{(960 - size) // 2}:{(600 - size) // 2}"],
                                  size, size)
                if img is not None:
                    res.append((n, img))
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_bytes(pickle.dumps(res))
    return res


def big_photo(out):
    cache = Path(out) / "webpenc" / "photo12mp.pkl"
    if cache.exists():
        return pickle.loads(cache.read_bytes())
    img = ffmpeg_bgra(["-i", WALL[1][1], "-vf", "scale=4000:3000"], 4000, 3000) if it.have_exe("ffmpeg") else None
    if img is not None:
        cache.parent.mkdir(parents=True, exist_ok=True)
        cache.write_bytes(pickle.dumps(img))
    return img


def extra_images(rng):
    out = []
    for w, h in [(1, 1), (2, 1), (1, 3), (5, 5), (15, 17), (16, 16), (17, 15), (31, 33), (129, 3), (3, 129)]:
        out.append((f"noise{w}x{h}", rng.integers(0, 256, (h, w, 4), dtype=np.uint8)))
    for k in (1, 2, 3, 4, 5, 16, 17, 255, 256, 257):
        pal = rng.integers(0, 256, (k, 4), dtype=np.uint8)
        out.append((f"palk{k}", pal[rng.integers(0, k, (23, 37))]))
    t = np.zeros((20, 30, 4), np.uint8)
    t[..., :3] = rng.integers(0, 256, (20, 30, 3))
    t[::2, :, 3] = 0
    t[1::2, :, 3] = 255
    out.append(("transparent-rgb", t))
    z = np.zeros((9, 9, 4), np.uint8)
    out.append(("all-zero", z))
    # palette whose first pixel is 0xFFFFFFFF (white, opaque) and all-white
    bw = np.zeros((11, 19, 4), np.uint8)
    bw[..., 3] = 255
    bw[(np.add.outer(np.arange(11), np.arange(19)) % 3) == 0, :3] = 255
    out.append(("white-first", bw))
    out.append(("all-white", np.full((5, 7, 4), 255, np.uint8)))
    return out


def riff_vp8(stream):
    return it.riff(b"WEBP", [it.riff_chunk(b"VP8 ", bytes(stream))])


def psnr3(a, b):
    return it.psnr(a[..., :3], b[..., :3])


def dec_ours(runner, files):
    res = runner.batch([it.Rec(it.OP_DECEX, 0, it.decex_input(f, C.IMGD_STRAIGHT), None, "dec") for f in files])
    out = []
    for r in res:
        if r is None or r[0] != 0:
            out.append(None)
        else:
            out.append(it.parse_decex(r[1])["px"])
    return out


# ================================================================== suites

def suite_lossless(runner, t, a, lib, cache):
    s = it.Suite("lossless", t)
    rng = np.random.default_rng(a.seed)
    imgs = wc.with_alphas(wc.corpus(rng, big=not a.quick), rng) + extra_images(rng)
    imgs += [(n, p) for n, p in photos(a.outdir, 128)]
    recs, exp = [], []
    for i, (n, px) in enumerate(imgs):
        efforts = [1, 5, 9] if i % 3 == 0 else ([3, 7] if i % 3 == 1 else [2, 4, 6, 8])
        for e in efforts:
            recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(flags=C.IOF_LOSSLESS, effort=e) + it.surface_input(px, C.FMT_ARGB),
                               None, f"{n} e{e}"))
            exp.append((px, False))
        if i % 4 == 0:
            recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(flags=C.IOF_LOSSLESS | C.IOF_WEBP_NOEXACT, effort=5) +
                               it.surface_input(px, C.FMT_ARGB), None, f"{n} noexact"))
            exp.append((px, True))
    # 16383 wide / high, a PARGB and an XRGB source
    wide = rng.integers(0, 256, (1, 16383, 4), dtype=np.uint8)
    wide[..., 3] = 255
    for px, nm in ((wide, "16383x1"), (wide.reshape(16383, 1, 4), "1x16383")):
        recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(flags=C.IOF_LOSSLESS) + it.surface_input(px, C.FMT_ARGB), None, nm))
        exp.append((px, False))
    res = it.run_suite(runner, s, [])
    res = runner.batch(recs)
    files = []
    for r, rec in zip(res, recs):
        files.append(bytes(r[1]) if r is not None and r[0] == 0 else b"")
    ldec = lib.decode(files)
    odec = dec_ours(runner, files)
    for rec, f, (px, noexact), ld, od in zip(recs, files, exp, ldec, odec):
        if not f:
            s.fail(f"{rec.name}: encode failed")
            continue
        for nm, d in (("libwebp", ld), ("ours", od)):
            if d is None:
                s.fail(f"{rec.name}: {nm} cannot decode")
                continue
            if noexact:
                vis = px[..., 3] != 0
                ok = np.array_equal(d[..., 3], px[..., 3]) and np.array_equal(d[vis], px[vis])
            else:
                ok = np.array_equal(d, px)
            s.check(ok, f"{rec.name}: {nm} decode differs")
            if not ok:
                fd = Path(a.outdir) / "webpenc" / "fail"
                fd.mkdir(parents=True, exist_ok=True)
                tag = rec.name.replace(" ", "_")
                (fd / f"{tag}.webp").write_bytes(f)
                (fd / f"{tag}.raw").write_bytes(np.ascontiguousarray(px).tobytes())
    s.notes.append(f"{len(recs)} encodes, {sum(len(f) for f in files)} bytes")
    return s


def suite_lossy(runner, t, a, lib, cache):
    s = it.Suite("lossy", t)
    rng = np.random.default_rng(a.seed + 1)
    imgs = wc.with_alphas(wc.corpus(rng, big=False), rng) + extra_images(rng) + [(n, p) for n, p in photos(a.outdir, 96)]
    recs, exp = [], []
    qs = [1, 20, 50, 75, 95, 100]
    for i, (n, px) in enumerate(imgs):
        q = qs[i % len(qs)]
        e = [1, 3, 5, 5, 7, 9][i % 6]
        pr = i % 6
        recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(quality=q, effort=e, preset=pr) + it.surface_input(px, C.FMT_ARGB),
                           None, f"{n} q{q} e{e} p{pr}"))
        exp.append(px)
    for shape in ((1, 16383), (16383, 1)):
        px = rng.integers(0, 256, shape + (4,), dtype=np.uint8)
        px[..., 3] = rng.integers(0, 2, shape) * 255
        recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(quality=60) + it.surface_input(px, C.FMT_ARGB), None,
                           f"edge{shape[1]}x{shape[0]} q60"))
        exp.append(px)
    res = runner.batch(recs)
    files = [bytes(r[1]) if r is not None and r[0] == 0 else b"" for r in res]
    ldec = lib.decode(files)
    odec = dec_ours(runner, files)
    for rec, f, px, ld, od in zip(recs, files, exp, ldec, odec):
        if not f:
            s.fail(f"{rec.name}: encode failed")
            continue
        if ld is None or od is None:
            s.fail(f"{rec.name}: not decodable (libwebp {ld is not None}, ours {od is not None})")
            continue
        s.check(np.array_equal(ld, od), f"{rec.name}: our decoder differs from libwebp")
        s.check(np.array_equal(ld[..., 3], px[..., 3]), f"{rec.name}: alpha not exact")
        opaque = px[..., 3] == 255
        if opaque.any() and px.shape[0] * px.shape[1] >= 64:
            p = it.psnr(ld[opaque][:, :3], px[opaque][:, :3])
            q = int(rec.name.split(" q")[1].split()[0])
            lim = 8 if q <= 20 else 12
            s.check(p >= lim or "noise" in rec.name, f"{rec.name}: PSNR {p:.1f}")
    s.notes.append(f"{len(recs)} files")
    return s


def suite_alpha(runner, t, a, lib, cache):
    s = it.Suite("alpha", t)
    rng = np.random.default_rng(a.seed + 2)
    recs = []
    for i, (w, h) in enumerate([(1, 1), (7, 3), (33, 17), (64, 64), (100, 37)]):
        for kind in ("random", "grad", "binary", "levels"):
            if kind == "random":
                al = rng.integers(0, 256, (h, w), dtype=np.uint8)
            elif kind == "grad":
                al = (np.add.outer(np.arange(h) * 7, np.arange(w) * 3) % 256).astype(np.uint8)
            elif kind == "binary":
                al = (rng.integers(0, 2, (h, w)) * 255).astype(np.uint8)
            else:
                al = (rng.integers(0, 5, (h, w)) * 60).astype(np.uint8)
            px = np.zeros((h, w, 4), np.uint8)
            px[..., 3] = al
            for e in (1, 5, 9):
                data = struct.pack("<4I", w, h, e, 0) + px.tobytes()

                def chk(res, out, al=al, w=w, h=h, e=e, kind=kind):
                    if res != 0:
                        return f"error {it.err_name(res)}"
                    r2, o2 = runner.one(OP_WALPHA, 0, struct.pack("<II", w, h) + bytes(out))
                    if r2 != 0:
                        return f"webp_alpha error {it.err_name(r2)}"
                    got = np.frombuffer(bytes(o2), np.uint8, count=w * h).reshape(h, w)
                    return None if np.array_equal(got, al) else "alpha differs"
                recs.append(it.Rec(OP_WEA, 0, data, chk, f"alpha {kind} {w}x{h} e{e}"))
    it.run_suite(runner, s, recs)
    return s


def suite_container(runner, t, a, lib, cache):
    s = it.Suite("container", t)
    rng = np.random.default_rng(a.seed + 3)
    prng = random.Random(a.seed + 3)
    op = it.rand_bgra(prng, 21, 13, "opaque")
    tr = it.rand_bgra(prng, 21, 13, "mixed")
    icc = bytes(rng.integers(0, 256, 131, dtype=np.uint8))          # odd length -> padding
    exif = it.exif_blob(orient=6)
    xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF/></x:xmpmeta>'
    cases = []
    for lossless in (0, 1):
        for px, nm in ((op, "opaque"), (tr, "alpha")):
            for blobs in ((b"", b"", b""), (icc, exif, xmp), (icc, b"", b""), (b"", exif, b""), (b"", b"", xmp)):
                cases.append((lossless, px, nm, blobs))
    recs = []
    for lossless, px, nm, (ic, ex, xm) in cases:
        fl = (C.IOF_LOSSLESS if lossless else 0) | C.IOF_EMBED_ICC | C.IOF_EMBED_EXIF | C.IOF_EMBED_XMP
        data = imgopt(quality=90, flags=fl) + struct.pack("<6I", 0, 0, 0, len(ic), len(ex), len(xm)) + ic + ex + xm + \
            it.surface_input(px, C.FMT_ARGB)
        recs.append(it.Rec(OP_ENCMETA, 0, data, None, f"{'ll' if lossless else 'lossy'} {nm} icc{len(ic)} exif{len(ex)} xmp{len(xm)}"))
    res = runner.batch(recs)
    files = [bytes(r[1]) if r is not None and r[0] == 0 else b"" for r in res]
    ldec = lib.decode(files)
    dres = runner.batch([it.Rec(it.OP_DECEX, 0, it.decex_input(f, C.IMGD_STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP), None, "d")
                         for f in files])
    for (lossless, px, nm, (ic, ex, xm)), rec, f, ld, dr in zip(cases, recs, files, ldec, dres):
        if not f:
            s.fail(f"{rec.name}: encode failed")
            continue
        riff = struct.unpack_from("<I", f, 4)[0]
        s.check(f[:4] == b"RIFF" and f[8:12] == b"WEBP" and riff == len(f) - 8 and len(f) % 2 == 0,
                f"{rec.name}: RIFF size {riff} / file {len(f)}")
        # chunk walk
        pos, chunks = 12, []
        while pos + 8 <= len(f):
            tag = f[pos:pos + 4]
            n = struct.unpack_from("<I", f, pos + 4)[0]
            chunks.append(tag)
            pos += 8 + n + (n & 1)
        s.check(pos == len(f), f"{rec.name}: chunk walk ends at {pos} of {len(f)}")
        alpha = nm == "alpha"
        ext = bool(ic or ex or xm) or (alpha and not lossless)
        want = [b"VP8X"] if ext else []
        if ic:
            want.append(b"ICCP")
        if alpha and not lossless:
            want.append(b"ALPH")
        want.append(b"VP8L" if lossless else b"VP8 ")
        if ex:
            want.append(b"EXIF")
        if xm:
            want.append(b"XMP ")
        s.check(chunks == want, f"{rec.name}: chunks {chunks} != {want}")
        if ext:
            flags = f[20]
            wf = (0x20 if ic else 0) | (0x10 if alpha else 0) | (0x08 if ex else 0) | (0x04 if xm else 0)
            s.check(flags == wf, f"{rec.name}: VP8X flags {flags:#x} != {wf:#x}")
            cw = int.from_bytes(f[24:27], "little") + 1
            ch = int.from_bytes(f[27:30], "little") + 1
            s.check((cw, ch) == (px.shape[1], px.shape[0]), f"{rec.name}: canvas {cw}x{ch}")
        s.check(ld is not None, f"{rec.name}: libwebp cannot decode")
        if dr is None or dr[0] != 0:
            s.fail(f"{rec.name}: our decoder fails")
            continue
        d = it.parse_decex(dr[1])
        s.check(d["icc"] == ic, f"{rec.name}: ICC not read back")
        s.check(d["xmp"] == xm, f"{rec.name}: XMP not read back")
        if ex:
            s.check(d["exif"] and d["meta"]["orient"] == 1, f"{rec.name}: EXIF orientation {d['meta']['orient']} (1 expected)")
        s.check(bool(d["meta"]["flags"] & C.IMF_LOSSLESS) == bool(lossless), f"{rec.name}: lossless flag")
        if lossless and ld is not None:
            s.check(np.array_equal(ld, px), f"{rec.name}: pixels")
    # capabilities of the codec (fmtinfo op): encode + decode, IOM fields
    r, out = runner.one(it.OP_FMTINFO, C.IMGF_WEBP, b"")
    if r >= 0 and len(out) >= 12:
        caps, iom = struct.unpack_from("<II", out, len(out) - 12)
        s.check(caps & C.IFC_ENCODE and caps & C.IFC_DECODE, f"caps {caps:#x}")
        want = C.IOM_QUALITY | C.IOM_LOSSLESS | C.IOM_EFFORT | C.IOM_ICC | C.IOM_EXIF | C.IOM_XMP | C.IOM_PRESET
        s.check(iom == want, f"IOM {iom:#x} != {want:#x}")
    else:
        s.notes.append(f"fmtinfo op: result {r}, {len(out)} bytes (not checked)")
    return s


def suite_errors(runner, t, a, lib, cache):
    s = it.Suite("errors", t)
    px = np.zeros((1, 16384, 4), np.uint8)
    recs = [it.Rec(it.OP_ENCODE, 0, imgopt(flags=C.IOF_LOSSLESS) + it.surface_input(px, C.FMT_ARGB), it.expect_err(C.ERR_DIMS), "lossless 16384"),
            it.Rec(it.OP_ENCODE, 0, imgopt() + it.surface_input(px, C.FMT_ARGB), it.expect_err(C.ERR_DIMS), "lossy 16384"),
            it.Rec(it.OP_ENCODE, 0, imgopt() + it.surface_input(px.reshape(16384, 1, 4), C.FMT_ARGB), it.expect_err(C.ERR_DIMS), "lossy 1x16384")]
    small = np.random.default_rng(1).integers(0, 256, (40, 40, 4), dtype=np.uint8)
    for op, hdr in ((OP_WEL, struct.pack("<4I", 40, 40, 5, 0x80000000)), (OP_WEV, struct.pack("<6I", 40, 40, 5, 0x80000000, 75, 0)),
                    (OP_WEA, struct.pack("<4I", 40, 40, 5, 0x80000000))):
        recs.append(it.Rec(op, 0, hdr + small.tobytes(), it.expect_err(C.ERR_CANCEL), f"cancel op {op}"))
    it.run_suite(runner, s, recs)
    return s


def suite_size(runner, t, a, lib, cache):
    s = it.Suite("size", t)
    rng = np.random.default_rng(5)
    imgs = photos(a.outdir, 512)
    for src in ("testsrc2", "smptehdbars", "mandelbrot", "cellauto", "rgbtestsrc"):
        arg = f"{src}=size=320x240" + (":random_seed=1" if src == "cellauto" else "")
        img = ffmpeg_bgra(["-f", "lavfi", "-i", arg], 320, 240) if it.have_exe("ffmpeg") else None
        if img is not None:
            imgs.append((src, img))
    for k in (2, 7, 30, 200):
        pal = rng.integers(0, 256, (k, 4), dtype=np.uint8)
        pal[:, 3] = 255
        idx = (np.add.outer(np.arange(150) // 7, np.arange(200) // 9) + rng.integers(0, 2, (150, 200))) % k
        imgs.append((f"pal{k}", pal[idx]))
    if imgs:
        ph = imgs[0][1].copy()
        yy, xx = np.mgrid[0:ph.shape[0], 0:ph.shape[1]]
        ph[..., 3] = np.clip(255 - np.hypot(yy - 256, xx - 256) * 1.2, 0, 255).astype(np.uint8)
        imgs.append(("photo-alpha", ph))
    # lossless
    refs = lib.encode([(px[..., [2, 1, 0, 3]], {"lossless": 1, "lossless_preset": 6, "exact": 1}) for n, px in imgs])
    res = runner.batch([it.Rec(it.OP_ENCODE, 0, imgopt(flags=C.IOF_LOSSLESS) + it.surface_input(px, C.FMT_ARGB), None, n)
                        for n, px in imgs])
    tot = totr = 0
    lg = 0.0
    rows = []
    for (n, px), r, ref in zip(imgs, res, refs):
        if r is None or r[0] != 0 or ref is None:
            s.fail(f"lossless {n}: encode failed")
            continue
        tot += len(r[1])
        totr += len(ref)
        lg += np.log(len(r[1]) / len(ref))
        rows.append(f"{n}:{len(r[1]) / len(ref):.3f}")
    if totr:
        ratio = tot / totr
        s.notes.append(f"lossless vs libwebp -z 6: total {ratio:.4f}, geomean {np.exp(lg / len(rows)):.4f}")
        s.notes.append("  " + " ".join(rows))
        s.check(ratio <= 1.10, f"lossless total ratio {ratio:.3f} > 1.10")
    # lossy
    lossy = [(n, px) for n, px in imgs if not n.startswith("pal")]
    specs, recs = [], []
    for n, px in lossy:
        o = px.copy()
        o[..., 3] = 255
        for q in (30, 75, 90):
            specs.append((o[..., [2, 1, 0, 3]], {"quality": q, "method": 4}))
            recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(quality=q) + it.surface_input(o, C.FMT_ARGB), None, f"{n} q{q}"))
    refs = lib.encode(specs)
    res = runner.batch(recs)
    files = [bytes(r[1]) if r is not None and r[0] == 0 else b"" for r in res]
    dec = lib.decode(files + refs)
    worst_p, worst_s = 99.0, 0.0
    sz = szr = 0
    for i, (rec, f, ref) in enumerate(zip(recs, files, refs)):
        n = rec.name.split()[0]
        o = dict(lossy)[n]
        d, dr = dec[i], dec[len(files) + i]
        if d is None or dr is None:
            s.fail(f"lossy {rec.name}: not decodable")
            continue
        dp = psnr3(d, o) - psnr3(dr, o)
        worst_p = min(worst_p, dp)
        worst_s = max(worst_s, len(f) / len(ref))
        sz += len(f)
        szr += len(ref)
        photo = n in dict(WALL) or n == "photo-alpha"
        if photo:
            s.check(dp >= -0.5 and len(f) <= 1.05 * len(ref), f"lossy {rec.name}: size {len(f) / len(ref):.3f}, dPSNR {dp:+.2f}")
        elif dp < -0.5 or len(f) > 1.05 * len(ref):
            s.notes.append(f"  (synthetic) {rec.name}: size {len(f) / len(ref):.3f}, dPSNR {dp:+.2f}")
    s.notes.insert(2, f"lossy vs libwebp (same quality, method 4): size {sz / max(1, szr):.4f} total, worst size "
                   f"{worst_s:.3f}, worst dPSNR {worst_p:+.2f} dB (photos must stay within -0.5 dB / 1.05)")
    return s


def suite_det(runner, t, a, lib, cache):
    s = it.Suite("det", t)
    rng = np.random.default_rng(a.seed + 7)
    imgs = [(n, p) for n, p in photos(a.outdir, 128)] + [("noise", rng.integers(0, 256, (37, 29, 4), dtype=np.uint8))]
    recs = []
    for n, px in imgs:
        for fl in (C.IOF_LOSSLESS, 0):
            for extra in (0, C.IOF_WEBP_1THREAD):
                recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(quality=80, flags=fl | extra) + it.surface_input(px, C.FMT_ARGB),
                                   None, f"{n} {fl} {extra}"))
    res = runner.batch(recs) + runner.batch(recs)
    half = len(recs)
    for i in range(0, half, 2):
        a0, a1, b0 = res[i], res[i + 1], res[half + i]
        ok = all(r is not None and r[0] == 0 for r in (a0, a1, b0))
        s.check(ok and bytes(a0[1]) == bytes(a1[1]) == bytes(b0[1]), f"{recs[i].name}: output differs between runs / threads")
    digest = hashlib.sha1(b"".join(bytes(r[1]) for r in res[:half] if r)).hexdigest()
    cache.setdefault("det", {})[t] = digest
    if len(cache["det"]) == 2:
        s.check(len(set(cache["det"].values())) == 1, f"Windows and Linux outputs differ: {cache['det']}")
        s.notes.append("Windows == Linux: " + ("yes" if len(set(cache["det"].values())) == 1 else "NO"))
    return s


def suite_perf(runner, t, a, lib, cache):
    s = it.Suite("perf", t)
    img = big_photo(a.outdir)
    if img is None:
        s.notes.append("no 12 MP photo (ffmpeg / wallpaper missing)")
        return s
    for nm, opt in (("lossless e5", imgopt(flags=C.IOF_LOSSLESS)), ("lossy q90", imgopt(quality=90)),
                    ("lossless e1", imgopt(flags=C.IOF_LOSSLESS, effort=1)), ("lossy q90 e1", imgopt(quality=90, effort=1))):
        r = runner.one(OP_WETIME, 1, opt + it.surface_input(img, C.FMT_ARGB))
        if r[0] != 0:
            s.fail(f"perf {nm}: {it.err_name(r[0])}")
            continue
        us, size = struct.unpack_from("<II", r[1], 0)
        s.notes.append(f"12 MP {nm}: {us / 1000:.0f} ms, {size} bytes")
        s.ok()
    return s


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
    lib = wc.Lib(out / "webpenc" / "libwebp_cache")
    cache = {}
    suites = []
    for t in targets:
        runner = it.Runner(t, out, work_name="webpenc")
        for name in only:
            suites.append(globals()["suite_" + name](runner, t, a, lib, cache))
    ok = it.report(suites, "webp_enc_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
