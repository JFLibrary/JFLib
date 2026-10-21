#!/usr/bin/env python3
"""GIF codec checks for jfguiLib (img/gif.s, img/quantize.s).

Runs apps/codectest in batch mode (ops 40..44, see apps/codectest/src/gif_ops.s)
on each target and compares every result with a reference:

  model  self-test of this file: the Python reference decoder (gif_model, same
         semantics as gif_decode) against the Python writer's construction
  dec    every variant the Python GIF writer produces: LZW minimum code sizes 1..8,
         clear codes (none first, early, deferred, full 4096-entry tables,
         giflib-style 4095), no EOI, data after EOI, more / fewer pixels than the
         frame, interlace, local / global / missing colour tables, GCE
         transparency, extensions, odd sizes, animations with offset frames,
         canvas growth, GIF87a; plus every error code
  fuzz   truncations, bit flips, byte changes, insertions and garbage -> exactly
         what gif_model says (error code or partial image); guarded input
         buffers turn any over-read into a crash
  quant  img_quantize: exact palettes, colour limits, nearest-colour mapping,
         transparency
  quantex img_quantize_ex (op 45): transparency thresholds 0..256 on straight and
         premultiplied input, IQF_WHITE, IQF_KEEPALPHA (exact with alpha, 0 when too
         many colours), dither levels 0..8; qcompat: level 0 / 8 bit-identical to
         img_quantize dither off / on, level 4 differs from both
  classic gif_encode / img_quantize / gif_decode byte-identical to the C0 reference binary
         (build/ca/ref/codectest_c0.exe, when present)
  camisc gif_decode_fl straight output (op 47), gif_encode_q (op 46) and img_encode
         with IO_DITHER / IO_ATHRESH, IMGMETA via op 261 (ICCRGBG1 profile, frames,
         NETSCAPE loops)
  enc    gif_encode: exact round trips (<= 256 colours), PSNR of photo-like
         images, transparency; decoded by gif_model and by our gif_decode
  ext    Windows GDI+ (System.Drawing via PowerShell) and ffmpeg (WSL):
         their GIFs decoded by us vs their own decoders, our GIFs decoded by
         them, Python-written edge cases (minimum code size 1, deferred clear,
         ...) decoded by them; our encoder output identical on both targets
  bench  timings (encode 1152x648 photo / drawing, decode 1920x1080) and the
         PSNR of ffmpeg's palettegen + paletteuse for comparison

  python tools/gif_check.py --target both --out build/agent-gif
  python tools/gif_check.py --target win --only dec,fuzz --no-build

Exit code 0 when everything passed.
"""

import argparse
import random
import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402
from codec_check import ERR_NAMES, Rec, Runner, Suite, build_targets, run_suite, unpremul_ref  # noqa: E402

OP_GIFDEC, OP_GIFENC, OP_QUANT, OP_GIFDEC_T, OP_GIFENC_T = 40, 41, 42, 43, 44
E_TRUNC, E_HDR, E_UNSUP, E_CHUNK, E_DIMS, E_NOMEM = -1, -2, -9, -11, -12, -8
FUZZ_MAX_PIXELS = 1 << 22


# ---------------------------------------------------------------- LZW

def lzw_codes(idx, mcs, first_clear=True, clear_every=0, limit=4096, deferred=0, eoi=True):
    """GIF LZW code list.  clear_every: clear after that many codes (early clear);
    limit: table size at which the encoder stops adding (4096 = full table,
    4095 = giflib); deferred: codes emitted with a full table before the clear."""
    clear = 1 << mcs
    codes = [clear] if first_clear else []
    table, nxt, since, held = {}, clear + 2, 0, 0
    data = bytes(idx)
    prefix = data[0]
    for c in data[1:]:
        k = (prefix << 8) | c
        code = table.get(k)
        if code is not None:
            prefix = code
            continue
        codes.append(prefix)
        since += 1
        if nxt < limit:
            table[k] = nxt
            nxt += 1
        elif held < deferred:
            held += 1
        else:
            codes.append(clear)
            table, nxt, since, held = {}, clear + 2, 0, 0
        if clear_every and since >= clear_every:
            codes.append(clear)
            table, nxt, since, held = {}, clear + 2, 0, 0
        prefix = c
    codes.append(prefix)
    if eoi:
        codes.append(clear + 1)
    return codes


def pack_codes(codes, mcs):
    """Codes -> bytes with the decoder's width rule (grow after a code when next >= 2^width)."""
    clear, eoi = 1 << mcs, (1 << mcs) + 1
    cs, nxt, prev = mcs + 1, clear + 2, False
    acc = nb = 0
    out = bytearray()
    for c in codes:
        if c >= 1 << cs:
            raise ValueError(f"code {c} does not fit {cs} bits")
        acc |= c << nb
        nb += cs
        while nb >= 8:
            out.append(acc & 255)
            acc >>= 8
            nb -= 8
        if c == clear:
            cs, nxt, prev = mcs + 1, clear + 2, False
        elif c != eoi:
            if prev and nxt < 4096:
                nxt += 1
            if nxt >= 1 << cs and cs < 12:
                cs += 1
            prev = True
    if nb:
        out.append(acc & 255)
    return bytes(out)


def lzw_decode(stream, mcs, total):
    """Reference LZW decoder: stops at EOI, at an undefined code, when the bits run out
    or when total pixels are there."""
    clear, eoi = 1 << mcs, (1 << mcs) + 1
    cs, nxt, prev = mcs + 1, clear + 2, None
    table = [b""] * 4096
    out = bytearray()
    bits = len(stream) * 8
    pos = acc = nb = bi = 0
    while len(out) < total:
        if pos + cs > bits:
            break
        while nb < cs:
            acc |= stream[bi] << nb
            bi += 1
            nb += 8
        code = acc & ((1 << cs) - 1)
        acc >>= cs
        nb -= cs
        pos += cs
        if code == clear:
            cs, nxt, prev = mcs + 1, clear + 2, None
            continue
        if code == eoi:
            break
        if code < clear:
            s = bytes((code,))
        elif code < nxt:
            s = table[code]
        elif code == nxt and prev is not None:
            s = prev + prev[:1]
        else:
            break
        if prev is not None and nxt < 4096:
            table[nxt] = prev + s[:1]
            nxt += 1
        if nxt >= 1 << cs and cs < 12:
            cs += 1
        out += s
        prev = s
    return bytes(out[:total])


# ---------------------------------------------------------------- reference decoder

def interlace_rows(h):
    return [y for (s, st) in ((0, 8), (4, 8), (2, 4), (1, 2)) for y in range(s, h, st)]


def make_lut(pal, trans):
    """pal: bytes r,g,b,... or list of (r, g, b) or None -> (256, 4) B,G,R,A premultiplied."""
    lut = np.zeros((256, 4), np.uint8)
    lut[:, 3] = 255
    if pal is None:
        lut[:, 0] = lut[:, 1] = lut[:, 2] = np.arange(256)
    else:
        if isinstance(pal, (bytes, bytearray)):
            pal = [tuple(pal[i:i + 3]) for i in range(0, len(pal), 3)]
        for i, (r, g, b) in enumerate(pal[:256]):
            lut[i, :3] = (b, g, r)
    if trans is not None and trans >= 0:
        lut[trans] = 0
    return lut


def gif_model(data, max_pixels=None):
    """gif_decode semantics in Python -> (result, (h, w, 4) B,G,R,A array or None, info)."""
    L = len(data)
    info = {"gce": False, "trans": -1}
    if L < 6:
        return E_TRUNC, None, info
    if data[:6] not in (b"GIF87a", b"GIF89a"):
        return E_HDR, None, info
    if L < 13:
        return E_TRUNC, None, info
    sw, sh, fl = struct.unpack_from("<HHB", data, 6)
    p = 13
    gct = None
    if fl & 0x80:
        n = 2 << (fl & 7)
        if L - p < 3 * n:
            return E_TRUNC, None, info
        gct = data[p:p + 3 * n]
        p += 3 * n
    info["gct_entries"] = len(gct) // 3 if gct else 0
    trans = -1
    while True:
        if p >= L:
            return E_TRUNC, None, info
        b = data[p]
        p += 1
        if b == 0x2C:
            break
        if b != 0x21:
            return E_CHUNK, None, info
        if p >= L:
            return E_TRUNC, None, info
        label = data[p]
        p += 1
        if label == 0xF9 and L - p >= 5 and data[p] >= 4:
            trans = data[p + 4] if data[p + 1] & 1 else -1
            info["gce"] = True
        while True:
            if p >= L:
                return E_TRUNC, None, info
            n = data[p]
            p += 1
            if n == 0:
                break
            if L - p < n:
                return E_TRUNC, None, info
            p += n
    if L - p < 9:
        return E_TRUNC, None, info
    fx, fy, fw, fh, ifl = struct.unpack_from("<HHHHB", data, p)
    p += 9
    if fw == 0 or fh == 0:
        fw, fh = sw, sh
    if fw == 0 or fh == 0:
        return E_DIMS, None, info
    cw, ch = max(sw, fx + fw), max(sh, fy + fh)
    if cw > 32768 or ch > 32768 or cw * ch > 1 << 28:
        return E_DIMS, None, info
    if max_pixels is not None and cw * ch > max_pixels:
        return None, None, info
    pal, mcs, stream = gct, 0, b""
    if ifl & 0x80:
        n = 2 << (ifl & 7)
        if L - p < 3 * n:
            p = L
        else:
            pal = data[p:p + 3 * n]
            p += 3 * n
    if p < L:
        mcs = data[p]
        p += 1
        if mcs < 1 or mcs > 8:
            return E_UNSUP, None, info
        chunks = []
        while p < L:
            n = data[p]
            p += 1
            if n == 0:
                break
            chunks.append(data[p:p + n])
            p += n
        stream = b"".join(chunks)
    info.update(trans=trans, frame=(fx, fy, fw, fh), mcs=mcs, pal=pal, interlace=bool(ifl & 0x40))
    img = np.zeros((ch, cw, 4), np.uint8)
    if mcs:
        idx = np.frombuffer(lzw_decode(stream, mcs, fw * fh), np.uint8)
        info["npix"] = len(idx)
        lut = make_lut(pal, trans)
        order = interlace_rows(fh) if ifl & 0x40 else range(fh)
        for k, y in enumerate(order):
            row = idx[k * fw:(k + 1) * fw]
            if len(row) == 0:
                break
            img[fy + y, fx:fx + len(row)] = lut[row]
    return 0, img, info


# ---------------------------------------------------------------- GIF writer

def ctable(pal, size=None):
    """(size field, bytes) of a colour table: next power of two >= entries (min 2)."""
    n = len(pal) if size is None else size
    bits = max(1, (max(n, 2) - 1).bit_length())
    out = bytearray()
    for i in range(1 << bits):
        out += bytes(pal[i] if i < len(pal) else (0, 0, 0))
    return bits - 1, bytes(out)


def subblocks(data, sizes=255, rng=None):
    out, p = bytearray(), 0
    while p < len(data):
        n = sizes if isinstance(sizes, int) else rng.randint(1, 255)
        chunk = data[p:p + n]
        out.append(len(chunk))
        out += chunk
        p += n
    out.append(0)
    return bytes(out)


def gce(trans=None, idx=0, size=4):
    body = bytes((1 if trans is not None else 0, 0, 0, trans if trans is not None else idx))
    body = (body + bytes(max(0, size - 4)))[:size]
    return bytes((0x21, 0xF9, size)) + body + b"\0"


def ext(label, *blocks):
    return bytes((0x21, label)) + b"".join(bytes((len(b),)) + b for b in blocks) + b"\0"


def frame_bytes(f, rng):
    out = bytearray(f.get("pre", b""))
    if f.get("trans") is not None or f.get("gce"):
        out += gce(f.get("trans"), f.get("gce_idx", 0))
    idx = f["idx"]
    ih, iw = idx.shape
    w, h = f.get("w", iw), f.get("h", ih)
    flags, tab = 0, b""
    if f.get("lct") is not None:
        sz, tab = ctable(f["lct"], f.get("lct_size"))
        flags |= 0x80 | sz
    if f.get("interlace"):
        flags |= 0x40
    out += struct.pack("<BHHHHB", 0x2C, f.get("x", 0), f.get("y", 0), w, h, flags) + tab
    mcs = f.get("mcs", 8)
    out.append(mcs)
    if "data" in f:
        out += f["data"]
        return bytes(out)
    if "stream" in f:
        stream = f["stream"]
    else:
        src = f.get("src", idx)
        rows = interlace_rows(src.shape[0]) if f.get("interlace") else range(src.shape[0])
        stream = pack_codes(lzw_codes(src[list(rows)].tobytes(), mcs, **f.get("lzw", {})), mcs)
    stream += f.get("extra", b"")
    out += subblocks(stream, f.get("blocks", 255), rng)
    return bytes(out)


def make_gif(spec, rng):
    f0 = spec["frames"][0]
    out = bytearray(spec.get("version", b"GIF89a"))
    flags, tab = 0, b""
    if spec.get("gct") is not None:
        sz, tab = ctable(spec["gct"], spec.get("gct_size"))
        flags = 0xF0 | sz
    sw, sh = spec.get("sw", f0["idx"].shape[1]), spec.get("sh", f0["idx"].shape[0])
    out += struct.pack("<HHBBB", sw, sh, flags, 0, 0) + tab + spec.get("head", b"")
    for f in spec["frames"]:
        out += frame_bytes(f, rng)
    out += spec.get("tail", b";")
    return bytes(out)


def expect_spec(spec):
    """Expected first frame of a writer spec, straight from the construction."""
    f = spec["frames"][0]
    idx = f["idx"]
    ih, iw = idx.shape
    sw, sh = spec.get("sw", iw), spec.get("sh", ih)
    x, y = f.get("x", 0), f.get("y", 0)
    cw, ch = max(sw, x + iw), max(sh, y + ih)
    pal = f.get("lct") if f.get("lct") is not None else spec.get("gct")
    lut = make_lut(pal, f.get("expect_trans", f.get("trans")))
    img = np.zeros((ch, cw, 4), np.uint8)
    img[y:y + ih, x:x + iw] = lut[idx]
    return img


# ---------------------------------------------------------------- checkers

def img_diff(got, exp):
    d = np.argwhere(np.any(got != exp, -1))
    y, x = d[0]
    return f"{len(d)} px differ, first ({x},{y}) got {got[y, x].tolist()} want {exp[y, x].tolist()}"


def parse_image(out):
    w, h = struct.unpack_from("<II", out)
    return np.frombuffer(out, np.uint8, offset=8).reshape(h, w, 4)


def expect_result(exp_res, exp_img):
    def chk(res, out):
        if res != exp_res:
            return f"result {res} ({ERR_NAMES.get(res)}), want {exp_res} ({ERR_NAMES.get(exp_res)})"
        if res != 0:
            return None
        got = parse_image(out)
        if got.shape != exp_img.shape:
            return f"size {got.shape[1]}x{got.shape[0]}, want {exp_img.shape[1]}x{exp_img.shape[0]}"
        if not np.array_equal(got, exp_img):
            return img_diff(got, exp_img)
        return None
    return chk


def psnr(a, b):
    """PSNR over RGB of (h, w, >=3) uint8 arrays (channel order irrelevant)."""
    d = a[..., :3].astype(np.float64) - b[..., :3].astype(np.float64)
    mse = float(np.mean(d * d))
    return 99.0 if mse == 0 else 10 * np.log10(255.0 * 255.0 / mse)


def premul_bgra(bgra):
    """Straight B,G,R,A with alpha 0/255 -> premultiplied (transparent -> 0)."""
    out = bgra.copy()
    out[bgra[..., 3] == 0] = 0
    return out


# ---------------------------------------------------------------- images

def rand_pal(nprng, n):
    return [tuple(int(v) for v in c) for c in nprng.integers(0, 256, (n, 3))]


def runs_img(nprng, w, h, n):
    """Index image with long runs (long LZW strings)."""
    out = np.zeros(w * h, np.uint8)
    p = 0
    while p < len(out):
        k = int(nprng.integers(1, 400))
        out[p:p + k] = nprng.integers(0, n)
        p += k
    return out.reshape(h, w)


def photo_image(w, h, seed=1):
    """Smooth photo-like RGB image (gradients, soft blobs, texture, mild noise)."""
    r = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    u, v = xx / w, yy / h
    img = np.empty((h, w, 3))
    img[..., 0] = 90 + 120 * v
    img[..., 1] = 140 + 60 * np.sin(np.pi * v)
    img[..., 2] = 230 - 150 * v
    for _ in range(8):
        cx, cy = r.uniform(0, 1, 2)
        s = r.uniform(0.05, 0.25)
        col = r.uniform(-120, 120, 3)
        img += np.exp(-((u - cx) ** 2 + (v - cy) ** 2) / (2 * s * s))[..., None] * col
    img += 12 * np.sin(u * 60 + np.sin(v * 25) * 3)[..., None] * np.array([1.0, 0.8, 0.6])
    img += r.normal(0, 2.0, img.shape)
    return np.clip(np.rint(img), 0, 255).astype(np.uint8)


def drawing_image(w, h, seed=2, ncol=20):
    """a drawing-program-like RGB image: flat shapes, outlines, thick lines, few colours."""
    r = np.random.default_rng(seed)
    pal = r.integers(0, 256, (ncol, 3)).astype(np.uint8)
    img = np.empty((h, w, 3), np.uint8)
    img[:] = (255, 255, 255)
    yy, xx = np.mgrid[0:h, 0:w]
    for _ in range(60):
        c = pal[r.integers(ncol)]
        t = int(r.integers(4))
        x0, y0 = int(r.integers(0, w)), int(r.integers(0, h))
        dx, dy = int(r.integers(4, max(5, w // 3))), int(r.integers(4, max(5, h // 3)))
        if t == 0:
            img[y0:y0 + dy, x0:x0 + dx] = c
        elif t == 1:
            m = ((xx - x0) / dx) ** 2 + ((yy - y0) / dy) ** 2 <= 1
            img[m] = c
        elif t == 2:
            x1, y1 = int(r.integers(0, w)), int(r.integers(0, h))
            ex, ey = x1 - x0, y1 - y0
            ln = max(1, ex * ex + ey * ey)
            tt = np.clip(((xx - x0) * ex + (yy - y0) * ey) / ln, 0, 1)
            m = (xx - x0 - tt * ex) ** 2 + (yy - y0 - tt * ey) ** 2 <= int(r.integers(1, 5)) ** 2
            img[m] = c
        else:
            img[y0:y0 + dy, x0:x0 + 2] = c
            img[y0:y0 + dy, x0 + dx - 2:x0 + dx] = c
            img[y0:y0 + 2, x0:x0 + dx] = c
            img[y0 + dy - 2:y0 + dy, x0:x0 + dx] = c
    for _ in range(w * h // 2000):
        gx, gy = int(r.integers(0, max(1, w - 7))), int(r.integers(0, max(1, h - 11)))
        g = r.integers(0, 2, (11, 7)).astype(bool)
        sub = img[gy:gy + 11, gx:gx + 7]
        sub[g[:sub.shape[0], :sub.shape[1]]] = (32, 32, 32)
    return img


def ui_image(w, h, seed=3):
    """Screenshot-like UI with a handful of theme colours and 3-level text."""
    r = np.random.default_rng(seed)
    img = np.empty((h, w, 3), np.uint8)
    img[:] = (243, 243, 243)
    img[:32] = (32, 32, 32)
    img[32:, :w // 5] = (230, 230, 230)
    for k in range(6):
        y = 40 + k * 28
        img[y:y + 22, 8:w // 5 - 8] = (0, 120, 212) if k == 2 else (230, 230, 230)
    for _ in range(12):
        x0, y0 = int(r.integers(w // 5, w - 60)), int(r.integers(40, h - 40))
        img[y0:y0 + 30, x0:x0 + 50] = (255, 255, 255)
        img[y0:y0 + 1, x0:x0 + 50] = (200, 200, 200)
    greys = np.array([(0, 0, 0), (96, 96, 96), (160, 160, 160)], np.uint8)
    for _ in range(w * h // 600):
        gx, gy = int(r.integers(0, max(1, w - 7))), int(r.integers(0, max(1, h - 11)))
        g = r.integers(0, 4, (11, 7))
        sub = img[gy:gy + 11, gx:gx + 7]
        for lv in range(3):
            sub[(g == lv + 1)[:sub.shape[0], :sub.shape[1]]] = greys[lv]
    return img


def to_bgra(rgb, alpha=None, garbage=None):
    h, w = rgb.shape[:2]
    a = np.full((h, w), 255, np.uint8) if alpha is None else alpha.astype(np.uint8)
    if garbage is not None:
        a = garbage.integers(0, 256, (h, w)).astype(np.uint8)
    return np.concatenate([rgb[..., ::-1], a[..., None]], -1).astype(np.uint8)


def premultiply(bgra):
    a = bgra[..., 3:4].astype(np.int64)
    rgb = (bgra[..., :3].astype(np.int64) * a + 127) // 255
    return np.concatenate([rgb, a], -1).astype(np.uint8)


def surf_bytes(fmt, arr):
    h, w = arr.shape[:2]
    return struct.pack("<III", w, h, fmt) + np.ascontiguousarray(arr).tobytes()


def surface_expect(fmt, arr):
    """What a GIF of the surface shows: (h, w, 4) B,G,R,A, opaque or 0."""
    if fmt == 2:
        g = arr.astype(np.uint8)
        return np.stack([g, g, g, np.full_like(g, 255)], -1)
    if fmt == 0:
        out = arr.copy()
        out[..., 3] = 255
        return out
    st = unpremul_ref(arr)                     # straight R,G,B,A
    out = np.stack([st[..., 2], st[..., 1], st[..., 0], np.full(st.shape[:2], 255, np.uint8)], -1)
    out[st[..., 3] < 128] = 0
    return out


def ncolours(img):
    v = img.astype(np.uint32)
    return len(np.unique((v[..., 0] | v[..., 1] << 8 | v[..., 2] << 16 | v[..., 3] << 24).ravel()))


# ---------------------------------------------------------------- decode cases

def dec_cases(rng):
    """-> list of (name, spec) for valid files (expected = construction)."""
    nr = np.random.default_rng(rng.randrange(1 << 30))
    cases = []

    def rimg(w, h, n):
        return nr.integers(0, n, (h, w)).astype(np.uint8)

    pal16, pal256 = rand_pal(nr, 16), rand_pal(nr, 256)
    for mcs in range(1, 9):
        n = 1 << mcs
        pal = rand_pal(nr, n)
        for (w, h) in ((1, 1), (2, 1), (7, 5), (64, 33), (150, 120)):
            cases.append((f"mcs{mcs} {w}x{h}", dict(gct=pal, frames=[dict(idx=rimg(w, h, n), mcs=mcs)])))
        cases.append((f"mcs{mcs} runs", dict(gct=pal, frames=[dict(idx=runs_img(nr, 211, 97, n), mcs=mcs)])))
        cases.append((f"mcs{mcs} interlaced", dict(gct=pal, frames=[dict(idx=rimg(37, 29, n), mcs=mcs,
                                                                           interlace=True)])))
    for mcs in (2, 5, 8):
        cases.append((f"2 colours, mcs{mcs}", dict(gct=rand_pal(nr, 2), frames=[dict(idx=rimg(40, 30, 2), mcs=mcs)])))
    cases.append(("mcs1 long 12-bit", dict(gct=rand_pal(nr, 2), frames=[dict(idx=rimg(300, 200, 2), mcs=1)])))
    for h in list(range(1, 18)) + [33, 64]:
        w = rng.choice([1, 3, 8, 13])
        cases.append((f"interlace {w}x{h}", dict(gct=pal16, frames=[dict(idx=rimg(w, h, 16), mcs=4, interlace=True)])))
    cases.append(("interlace 150x120", dict(gct=pal256, frames=[dict(idx=rimg(150, 120, 256), interlace=True)])))
    # colour tables
    cases.append(("local table only", dict(frames=[dict(idx=rimg(20, 10, 16), lct=pal16, mcs=4)])))
    cases.append(("local overrides global", dict(gct=pal256, frames=[dict(idx=rimg(20, 10, 16), lct=pal16, mcs=4)])))
    cases.append(("local 4 < index", dict(gct=pal256, frames=[dict(idx=rimg(30, 10, 16), lct=pal16[:4], mcs=4)])))
    cases.append(("no colour table (grey)", dict(frames=[dict(idx=rimg(30, 20, 256))])))
    cases.append(("index past table", dict(gct=pal16[:4], frames=[dict(idx=rimg(40, 20, 256))])))
    cases.append(("global 2 entries mcs8", dict(gct=pal16[:2], frames=[dict(idx=rimg(40, 20, 256))])))
    # transparency
    for t in (0, 7, 15):
        cases.append((f"transparent {t}", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 16), mcs=4, trans=t)])))
    cases.append(("transparent 200 past table", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 256), trans=200)])))
    cases.append(("GCE without flag", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 16), mcs=4, gce=True,
                                                                    gce_idx=3)])))
    cases.append(("last GCE wins", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 16), mcs=4, pre=gce(3),
                                                                 trans=5)])))
    cases.append(("GCE then comment", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 16), mcs=4,
                                                                    pre=gce(9) + ext(0xFE, b"hello"),
                                                                    expect_trans=9)])))
    cases.append(("GCE size 5", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 16), mcs=4,
                                                              pre=gce(6, size=5), expect_trans=6)])))
    cases.append(("GCE size 3 ignored", dict(gct=pal16, frames=[dict(idx=rimg(24, 16, 16), mcs=4,
                                                                      pre=gce(6, size=3), expect_trans=None)])))
    # LZW variants on a 256-colour image (many table fills)
    big = rimg(256, 200, 256)
    for nm, opts in (("no first clear", dict(first_clear=False)), ("early clear 100", dict(clear_every=100)),
                     ("clear every code", dict(clear_every=1)), ("early clear 1000", dict(clear_every=1000)),
                     ("limit 4095 (giflib)", dict(limit=4095)), ("limit 3000", dict(limit=3000)),
                     ("deferred 1", dict(deferred=1)), ("deferred 50", dict(deferred=50)),
                     ("deferred forever", dict(deferred=10 ** 9)), ("no EOI", dict(eoi=False)),
                     ("full 4096 tables", dict())):
        cases.append((f"lzw {nm}", dict(gct=pal256, frames=[dict(idx=big, lzw=opts)])))
    cases.append(("lzw mcs4 deferred", dict(gct=pal16, frames=[dict(idx=rimg(300, 200, 16), mcs=4,
                                                                      lzw=dict(deferred=3000))])))
    cases.append(("lzw mcs2 no clear, deferred", dict(gct=pal16[:4], frames=[dict(
        idx=rimg(300, 200, 4), mcs=2, lzw=dict(first_clear=False, deferred=10 ** 9))])))
    cases.append(("data after EOI", dict(gct=pal16, frames=[dict(idx=rimg(40, 30, 16), mcs=4,
                                                                  extra=bytes(range(255)) * 2)])))
    more = rimg(40, 35, 16)
    cases.append(("more pixels than frame", dict(gct=pal16, frames=[dict(idx=more[:30], src=more, mcs=4)])))
    cases.append(("more pixels, no EOI", dict(gct=pal16, frames=[dict(idx=more[:30], src=more, mcs=4,
                                                                       lzw=dict(eoi=False))])))
    cases.append(("random sub-blocks", dict(gct=pal256, frames=[dict(idx=rimg(120, 80, 256), blocks="rand")])))
    cases.append(("1-byte sub-blocks", dict(gct=pal16, frames=[dict(idx=rimg(50, 20, 16), mcs=4, blocks=1)])))
    for (w, h) in ((1, 300), (300, 1), (3, 7), (255, 3), (257, 2), (1000, 2)):
        cases.append((f"odd {w}x{h}", dict(gct=pal256, frames=[dict(idx=rimg(w, h, 256))])))
    # extensions and animations
    exts = (ext(0xFE, b"a comment", b"more") + ext(0xFF, b"NETSCAPE2.0", b"\x01\x00\x00")
            + ext(0x01, bytes(12), b"plain text") + ext(0x77, b"unknown label") + ext(0xFE))
    cases.append(("extensions", dict(gct=pal16, head=exts, frames=[dict(idx=rimg(30, 20, 16), mcs=4)])))
    f2 = dict(idx=rimg(10, 10, 16), mcs=4, x=3, y=4, pre=gce(2) + ext(0xFE, b"x"))
    cases.append(("animated, offset frame", dict(gct=pal16, sw=60, sh=40, frames=[
        dict(idx=rimg(20, 12, 16), mcs=4, x=7, y=5, trans=1), f2])))
    cases.append(("animated, second frame bigger", dict(gct=pal16, sw=30, sh=20, frames=[
        dict(idx=rimg(30, 20, 16), mcs=4), dict(idx=rimg(50, 50, 16), mcs=4)])))
    cases.append(("frame larger than screen", dict(gct=pal16, sw=15, sh=8, frames=[
        dict(idx=rimg(20, 10, 16), mcs=4, x=10, y=5)])))
    cases.append(("screen 0x0", dict(gct=pal16, sw=0, sh=0, frames=[dict(idx=rimg(20, 10, 16), mcs=4, x=4, y=2)])))
    cases.append(("frame w=0 -> screen", dict(gct=pal16, sw=13, sh=9, frames=[dict(idx=rimg(13, 9, 16), mcs=4,
                                                                                    w=0)])))
    cases.append(("frame h=0 -> screen", dict(gct=pal16, sw=13, sh=9, frames=[dict(idx=rimg(13, 9, 16), mcs=4,
                                                                                    h=0)])))
    cases.append(("GIF87a", dict(version=b"GIF87a", gct=pal16, frames=[dict(idx=rimg(20, 10, 16), mcs=4)])))
    cases.append(("no trailer", dict(gct=pal16, tail=b"", frames=[dict(idx=rimg(20, 10, 16), mcs=4)])))
    cases.append(("garbage after image", dict(gct=pal16, tail=b"\x99garbage", frames=[
        dict(idx=rimg(20, 10, 16), mcs=4)])))
    return cases


def model_cases(rng):
    """Valid-but-partial files: expected from gif_model (checked against hand-made expectations)."""
    nr = np.random.default_rng(rng.randrange(1 << 30))
    pal = rand_pal(nr, 16)
    idx = nr.integers(0, 16, (30, 40)).astype(np.uint8)
    out = []
    few = idx[:12]
    # stream for 12 rows + 5 pixels only, EOI; frame says 30 rows
    part = np.concatenate([few.ravel(), idx[12, :5]])
    stream = pack_codes(lzw_codes(part.tobytes(), 4), 4)
    out.append(("fewer pixels than frame", dict(gct=pal, frames=[dict(idx=idx, mcs=4, stream=stream)]),
                (12 * 40 + 5)))
    out.append(("fewer pixels, interlaced", dict(gct=pal, frames=[dict(idx=idx, mcs=4, stream=stream,
                                                                        interlace=True)]), 12 * 40 + 5))
    out.append(("empty data", dict(gct=pal, frames=[dict(idx=idx, mcs=4, data=b"\0")]), 0))
    out.append(("clear + EOI only", dict(gct=pal, frames=[dict(idx=idx, mcs=4, stream=pack_codes([16, 17], 4))]), 0))
    bad = lzw_codes(idx.tobytes(), 4)
    bad = bad[:51] + [100]                          # 7-bit code > next (67): decoding stops there
    out.append(("undefined code", dict(gct=pal, frames=[dict(idx=idx, mcs=4, stream=pack_codes(bad, 4)
                                                             + b"\xff\xff")]), None))
    return out


def error_cases():
    lsd = struct.pack("<HHBBB", 10, 10, 0, 0, 0)
    hdr = b"GIF89a" + lsd
    img1 = b",\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0;"
    cases = [
        ("empty", b"", E_TRUNC), ("GIF8", b"GIF8", E_TRUNC), ("GIF89a only", b"GIF89a", E_TRUNC),
        ("short LSD", b"GIF89a\x0a\0\x0a\0\0\0", E_TRUNC), ("bad version", b"GIF89b" + lsd + img1, E_HDR),
        ("PNG", b"\x89PNG\r\n\x1a\n" + bytes(20), E_HDR), ("GIF90a", b"GIF90a" + lsd + img1, E_HDR),
        ("gct truncated", b"GIF89a" + struct.pack("<HHBBB", 10, 10, 0x87, 0, 0) + bytes(700), E_TRUNC),
        ("no blocks", hdr, E_TRUNC), ("trailer only", hdr + b";", E_CHUNK), ("zero byte block", hdr + b"\0", E_CHUNK),
        ("garbage block", hdr + b"X" + img1, E_CHUNK), ("extension label only", hdr + b"!", E_TRUNC),
        ("extension truncated", hdr + b"!\xfe\x05ab", E_TRUNC), ("extension no terminator", hdr + b"!\xfe\x02ab",
                                                                   E_TRUNC),
        ("GCE truncated", hdr + b"!\xf9\x04\x01\0", E_TRUNC),
        ("descriptor truncated", hdr + b",\0\0\0\0\x05", E_TRUNC),
        ("0x0 frame, 0x0 screen", b"GIF89a" + struct.pack("<HHBBB", 0, 0, 0, 0, 0) + b",\0\0\0\0\0\0\0\0\0\x02\0;",
         E_DIMS),
        ("screen 40000 wide", b"GIF89a" + struct.pack("<HHBBB", 40000, 1, 0, 0, 0) + img1, E_DIMS),
        ("frame at x 65000", hdr + b"," + struct.pack("<HHHHB", 65000, 0, 10, 10, 0) + b"\x02\0;", E_DIMS),
        ("20000x20000", hdr + b"," + struct.pack("<HHHHB", 0, 0, 20000, 20000, 0) + b"\x02\0;", E_DIMS),
        ("32769 tall", hdr + b"," + struct.pack("<HHHHB", 0, 0, 1, 32769, 0) + b"\x02\0;", E_DIMS),
        ("mcs 0", hdr + b",\0\0\0\0\x01\0\x01\0\0\x00\x02\x44\x01\0;", E_UNSUP),
        ("mcs 9", hdr + b",\0\0\0\0\x01\0\x01\0\0\x09\x02\x44\x01\0;", E_UNSUP),
        ("mcs 12", hdr + b",\0\0\0\0\x01\0\x01\0\0\x0c\x02\x44\x01\0;", E_UNSUP),
    ]
    empty = np.zeros((10, 10, 4), np.uint8)
    ok = [("local table truncated", hdr + b",\0\0\0\0\x05\0\x05\0\x87" + bytes(100), empty),
          ("no code size byte", hdr + b",\0\0\0\0\x05\0\x05\0\0", empty),
          ("32768 wide ok", b"GIF89a" + struct.pack("<HHBBB", 32768, 1, 0, 0, 0) + img1,
           None)]
    return cases, ok


def suite_model(rng):
    """Python-only self-test of gif_model against the writer's construction."""
    s = Suite("model")
    for name, spec in dec_cases(random.Random(rng.random())):
        data = make_gif(spec, rng)
        res, img, _ = gif_model(data)
        exp = expect_spec(spec)
        if res != 0 or img.shape != exp.shape or not np.array_equal(img, exp):
            s.fail(f"{name}: model disagrees ({res})")
        else:
            s.ok()
    for name, spec, npix in model_cases(random.Random(rng.random())):
        res, img, info = gif_model(make_gif(spec, rng))
        if res != 0 or (npix is not None and info.get("npix", 0) != npix):
            s.fail(f"{name}: model res {res} npix {info.get('npix')} want {npix}")
        else:
            s.ok()
    return s


def suite_dec(rng):
    recs = []
    for name, spec in dec_cases(rng):
        recs.append(Rec(OP_GIFDEC, 0, make_gif(spec, rng), expect_result(0, expect_spec(spec)), name))
    for name, spec, _ in model_cases(rng):
        data = make_gif(spec, rng)
        res, img, _ = gif_model(data)
        recs.append(Rec(OP_GIFDEC, 0, data, expect_result(res, img), name))
    errs, oks = error_cases()
    for name, data, code in errs:
        recs.append(Rec(OP_GIFDEC, 0, data, expect_result(code, None), "error: " + name))
    for name, data, img in oks:
        res, mimg, _ = gif_model(data)
        if img is not None and not np.array_equal(mimg, img):
            raise AssertionError(f"model disagrees on {name}")
        recs.append(Rec(OP_GIFDEC, 0, data, expect_result(res, mimg), "edge: " + name))
    return recs


def mutate(rng, data):
    d = bytearray(data)
    k = rng.randrange(7)
    if k == 0:
        return bytes(d[:rng.randrange(len(d))])
    if k == 1:
        for _ in range(rng.randint(1, 4)):
            i = rng.randrange(len(d))
            d[i] ^= 1 << rng.randrange(8)
    elif k == 2:
        for _ in range(rng.randint(1, 3)):
            d[rng.randrange(len(d))] = rng.randrange(256)
    elif k == 3:
        i = rng.randrange(len(d))
        d[i:i] = bytes(rng.randrange(256) for _ in range(rng.randint(1, 8)))
    elif k == 4:
        i = rng.randrange(len(d))
        del d[i:i + rng.randint(1, 8)]
    elif k == 5:                                   # header / descriptor fields
        i = rng.choice([6, 7, 8, 9, 10] + [j for j in range(13, min(len(d), 900)) if d[j] == 0x2C][:3])
        if i < len(d):
            d[i] = rng.choice([0, 1, 0x7F, 0x80, 0xFF, rng.randrange(256)])
            if i + 1 < len(d) and rng.random() < 0.5:
                d[i + 1] = rng.randrange(256)
    else:
        i = rng.randrange(len(d))
        n = rng.randint(1, 64)
        d[i:i + n] = bytes(rng.randrange(256) for _ in range(n))
    return bytes(d)


def suite_fuzz(rng, count):
    recs = []
    nr = np.random.default_rng(rng.randrange(1 << 30))
    base = []
    for name, spec in dec_cases(random.Random(rng.random())):
        data = make_gif(spec, rng)
        if len(data) < 4000:
            base.append((name, data))
    small = sorted(base, key=lambda t: len(t[1]))
    # every truncation of a few files
    for name, data in [small[len(small) // 3], small[len(small) // 2], small[-1]]:
        for n in range(len(data)):
            res, img, _ = gif_model(data[:n])
            recs.append(Rec(OP_GIFDEC, 0, data[:n], expect_result(res, img), f"trunc {name} @{n}"))
    for i in range(count):
        name, data = base[rng.randrange(len(base))]
        m = data
        for _ in range(rng.choice([1, 1, 1, 2, 3])):
            if len(m) < 2:
                break
            m = mutate(rng, m)
        res, img, _ = gif_model(m, FUZZ_MAX_PIXELS)
        if res is None:
            continue
        recs.append(Rec(OP_GIFDEC, 0, m, expect_result(res, img), f"fuzz {i} ({name})"))
    for i in range(200):                           # garbage behind a valid header
        g = b"GIF89a" + nr.integers(0, 256, rng.randint(0, 600)).astype(np.uint8).tobytes()
        res, img, _ = gif_model(g, FUZZ_MAX_PIXELS)
        if res is not None:
            recs.append(Rec(OP_GIFDEC, 0, g, expect_result(res, img), f"garbage {i}"))
    return recs


# ---------------------------------------------------------------- quantizer

def quant_parse(out, n):
    cnt, tr = struct.unpack_from("<ii", out)
    pal = np.frombuffer(out, np.uint32, 256, 8)
    idx = np.frombuffer(out, np.uint8, n, 8 + 1024)
    return cnt, tr, pal, idx


def pal_bgra(pal):
    return pal.view(np.uint8).reshape(256, 4)


def suite_quant(rng):
    recs = []
    nr = np.random.default_rng(rng.randrange(1 << 30))

    def add(name, fmt, arr, maxc, dither, check):
        recs.append(Rec(OP_QUANT, maxc | dither << 16, surf_bytes(fmt, arr), check, name))

    def exact_check(fmt, arr, maxc):
        exp = surface_expect(fmt, arr)
        flat = exp.reshape(-1, 4).view(np.uint32).ravel()
        order = []
        seen = {}
        for v in flat:
            if v not in seen:
                seen[v] = len(order)
                order.append(v)
        want_idx = np.array([seen[v] for v in flat], np.uint8)
        want_tr = seen.get(np.uint32(0), -1)

        def chk(res, out):
            if res != 0:
                return f"record {res}"
            cnt, tr, pal, idx = quant_parse(out, len(flat))
            if cnt != len(order):
                return f"count {cnt}, want {len(order)}"
            if tr != want_tr:
                return f"transparent index {tr}, want {want_tr}"
            # palette: straight 0xAARRGGBB in order of first appearance (transparent = 0)
            got = pal[:cnt]
            want = np.array(order, np.uint32)
            if not np.array_equal(got, want):
                return f"palette differs: {got[:4]} vs {want[:4]}"
            if np.any(pal[cnt:]):
                return "entries past the count not 0"
            if not np.array_equal(idx, want_idx):
                return f"indices differ ({int(np.sum(idx != want_idx))})"
            return None
        return chk

    def limit_check(fmt, arr, maxc, dither, min_psnr):
        exp = surface_expect(fmt, arr)
        flat = exp.reshape(-1, 4)
        tmask = flat[:, 3] == 0

        def chk(res, out):
            if res != 0:
                return f"record {res}"
            cnt, tr, pal, idx = quant_parse(out, len(flat))
            if not 1 <= cnt <= maxc:
                return f"count {cnt} not in 1..{maxc}"
            if np.any(idx >= cnt):
                return "index >= count"
            pb = pal_bgra(pal)
            if tmask.any():
                if tr < 0 or pal[tr] != 0:
                    return f"transparent index {tr} / entry {int(pal[tr]) if tr >= 0 else -1:#x}"
                if not np.array_equal(idx == tr, tmask):
                    return "transparent pixels do not map to the transparent entry"
            elif tr != -1:
                return f"transparent index {tr} without transparent pixels"
            opq = ~tmask
            if np.any(pb[idx[opq], 3] != 255):
                return "opaque pixel mapped to a non-opaque entry"
            got = pb[idx].astype(np.int64)
            src = flat.astype(np.int64)
            if not dither:                          # nearest colour up to the 64^3 cell approximation
                cand = pb[:cnt][pb[:cnt, 3] == 255].astype(np.int64)
                d_all = ((src[opq, None, :3] - cand[None, :, :3]) ** 2).sum(-1)
                dmin = np.sqrt(d_all.min(1))
                dgot = np.sqrt(((src[opq, :3] - got[opq, :3]) ** 2).sum(-1))
                if np.any(dgot > dmin + 7.0):
                    k = int(np.argmax(dgot - dmin))
                    return f"pixel {k} maps {dgot[k]:.1f} away, nearest {dmin[k]:.1f}"
            p = psnr(got[opq], src[opq])
            if p < min_psnr:
                return f"PSNR {p:.2f} dB < {min_psnr}"
            return None
        return chk

    # exact palettes
    for (w, h, ncol) in ((1, 1, 1), (7, 3, 5), (33, 17, 200), (64, 64, 256)):
        pal = nr.integers(0, 256, (ncol, 3)).astype(np.uint8)
        rgb = pal[nr.integers(0, ncol, (h, w))]
        arr = to_bgra(rgb, garbage=nr)
        add(f"exact {w}x{h} {ncol} XRGB", 0, arr, 256, 1, exact_check(0, arr, 256))
    g = (np.arange(256 * 3).reshape(3, 256) % 256).astype(np.uint8)
    add("exact A8 256 greys", 2, g, 256, 1, exact_check(2, g, 256))
    a = to_bgra(drawing_image(80, 50, 5, 10))
    a[10:20, 10:40, 3] = 0
    a[30:32] = (0, 0, 0, 0)
    pm = premultiply(a)
    add("exact PARGB with transparency", 1, pm, 256, 1, exact_check(1, pm, 256))
    nc = ncolours(surface_expect(1, pm))
    add(f"exact max {nc} (incl. transparent)", 1, pm, nc, 1, exact_check(1, pm, nc))
    add(f"max {nc - 1} -> quantized", 1, pm, nc - 1, 0, limit_check(1, pm, nc - 1, 0, 10))
    allt = np.zeros((5, 9, 4), np.uint8)
    add("all transparent", 1, allt, 256, 1, exact_check(1, allt, 256))
    # limited palettes (Wu)
    ph = photo_image(160, 120, 7)
    for maxc, dth, mp in ((2, 0, 10), (16, 0, 20), (16, 1, 18), (64, 0, 26), (255, 0, 30), (256, 0, 30), (256, 1, 28)):
        arr = to_bgra(ph)
        add(f"photo max {maxc} dither {dth}", 0, arr, maxc, dth, limit_check(0, arr, maxc, dth, mp))
    pha = to_bgra(ph)
    yy, xx = np.mgrid[0:120, 0:160]
    pha[..., 3] = np.clip((xx - 20) * 3, 0, 255)
    pm = premultiply(pha)
    for maxc, dth in ((256, 1), (256, 0), (8, 1), (2, 0)):
        add(f"photo alpha max {maxc} dither {dth}", 1, pm, maxc, dth, limit_check(1, pm, maxc, dth, 8))
    d = drawing_image(90, 70, 9, 30)
    arr = to_bgra(d)
    add("drawing 30 colours max 8", 0, arr, 8, 0, limit_check(0, arr, 8, 0, 5))
    return recs


# ---------------------------------------------------------------- encoder

def enc_surfaces(rng):
    """-> list of (name, fmt, arr, exact) test surfaces for gif_encode."""
    nr = np.random.default_rng(rng.randrange(1 << 30))
    out = []
    for (w, h) in ((1, 1), (1, 7), (7, 1), (33, 17), (300, 200)):
        d = drawing_image(w, h, int(nr.integers(1 << 20)), 12)
        out.append((f"drawing {w}x{h} XRGB", 0, to_bgra(d, garbage=nr), True))
    d = drawing_image(97, 61, 11, 24)
    out.append(("drawing PARGB opaque", 1, to_bgra(d), True))
    a = to_bgra(d)
    a[5:30, 5:50, 3] = 0
    a[:, ::7, 3] = 0
    out.append(("drawing PARGB binary alpha", 1, premultiply(a), True))
    a = to_bgra(d)
    a[..., 3] = np.where((np.arange(97)[None, :] // 7) % 2 == 0, 200, 60)
    pm = premultiply(a)
    out.append(("drawing PARGB partial alpha", 1, pm, ncolours(surface_expect(1, pm)) <= 256))
    g = np.tile(np.arange(256, dtype=np.uint8), (20, 1))
    out.append(("A8 256 greys", 2, g, True))
    out.append(("A8 two values", 2, (np.arange(60).reshape(6, 10) % 2 * 255).astype(np.uint8), True))
    pal = nr.integers(0, 256, (256, 3)).astype(np.uint8)
    rgb = pal[np.arange(64 * 64).reshape(64, 64) % 256]
    out.append(("exactly 256 colours", 0, to_bgra(rgb), True))
    a = to_bgra(pal[np.arange(64 * 64).reshape(64, 64) % 255])
    a[0, 0, 3] = 0
    out.append(("255 colours + transparent", 1, premultiply(a), True))
    a = to_bgra(rgb)
    a[0, 0, 3] = 0
    out.append(("256 colours + transparent (quantized)", 1, premultiply(a), False))
    out.append(("one colour", 0, to_bgra(np.full((9, 13, 3), 77, np.uint8)), True))
    out.append(("all transparent", 1, np.zeros((8, 8, 4), np.uint8), True))
    out.append(("random 200 colours 256x256", 0, to_bgra(pal[:200][nr.integers(0, 200, (256, 256))]), True))
    out.append(("ui screenshot 640x400", 0, to_bgra(ui_image(640, 400)), True))
    out.append(("drawing 1152x648", 0, to_bgra(drawing_image(1152, 648, 4, 24)), True))
    out.append(("photo 320x240", 0, to_bgra(photo_image(320, 240, 3)), False))
    out.append(("photo 1152x648", 0, to_bgra(photo_image(1152, 648, 1)), False))
    ph = to_bgra(photo_image(200, 150, 5))
    yy, xx = np.mgrid[0:150, 0:200]
    ph[..., 3] = np.where((xx - 100) ** 2 + (yy - 75) ** 2 < 60 ** 2, 255, np.clip(xx * 2, 0, 255))
    out.append(("photo with alpha", 1, premultiply(ph), False))
    return out


def gif_structure(data):
    """(colour table entries, has GCE, trans index) of a single-image GIF of ours."""
    fl = data[10]
    n = 2 << (fl & 7) if fl & 0x80 else 0
    p = 13 + 3 * n
    has_gce = data[p] == 0x21 and data[p + 1] == 0xF9
    return n, has_gce, data[p + 6] if has_gce else -1


def run_enc(runner, rng, suite):
    """Encode every test surface, check with gif_model, re-decode with gif_decode.
    Returns [(name, gif bytes, expected image, exact)] for the ext suite."""
    surfs = enc_surfaces(rng)
    recs = [Rec(OP_GIFENC, 0, surf_bytes(f, a), None, n) for n, f, a, _ in surfs]
    crashes = []
    results = runner.batch(recs, crashes)
    made = []
    for (name, fmt, arr, exact), r in zip(surfs, results):
        if r is None:
            suite.fail(f"CRASH encode {name}")
            continue
        res, gif = r
        if res != 0:
            suite.fail(f"{name}: encode result {res}")
            continue
        exp = surface_expect(fmt, arr)
        mres, img, info = gif_model(gif)
        if mres != 0 or img.shape != exp.shape:
            suite.fail(f"{name}: reference decoder says {mres}")
            continue
        n, has_gce, tr = gif_structure(gif)
        want_tr = bool(np.any(exp[..., 3] == 0))
        msg = None
        if gif[:6] != b"GIF89a":
            msg = "not GIF89a"
        elif has_gce != want_tr:
            msg = f"GCE {has_gce}, transparency {want_tr}"
        elif not np.array_equal(img[..., 3] == 0, exp[..., 3] == 0):
            msg = "transparency differs"
        elif exact:
            nc = ncolours(exp)
            want_n = max(2, 1 << (nc - 1).bit_length())
            if not np.array_equal(img, exp):
                msg = img_diff(img, exp)
            elif n != want_n:
                msg = f"colour table {n} entries, want {want_n} for {nc} colours"
        else:
            opq = exp[..., 3] == 255
            p = psnr(img[opq], exp[opq])
            suite.notes.append(f"{name}: {len(gif)} bytes, PSNR {p:.2f} dB, table {n}")
            if p < 28.0:
                msg = f"PSNR {p:.2f} dB < 28"
        if msg:
            suite.fail(f"{name}: {msg}")
        else:
            suite.ok()
        made.append((name, gif, img, exact))
    # our decoder on our files
    recs = [Rec(OP_GIFDEC, 0, g, expect_result(0, img), "re-decode " + n) for n, g, img, _ in made]
    for r, res in zip(recs, runner.batch(recs, crashes)):
        msg = "crash" if res is None else r.check(*res)
        if msg:
            suite.fail(f"{r.name}: {msg}")
        else:
            suite.ok()
    for c in crashes[:5]:
        suite.msgs.append("crash detail: " + c)
    return made


# ---------------------------------------------------------------- external tools

PS_SCRIPT = r'''
param([string]$jobs)
Add-Type -AssemblyName System.Drawing
$fmt = [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
foreach ($line in [IO.File]::ReadAllLines($jobs)) {
  if ($line -eq '') { continue }
  $f = $line.Split('|')
  try {
    if ($f[0] -eq 'enc') {
      $w = [int]$f[1]; $h = [int]$f[2]
      $bytes = [IO.File]::ReadAllBytes($f[3])
      $bmp = New-Object System.Drawing.Bitmap($w, $h, $fmt)
      $rect = New-Object System.Drawing.Rectangle(0, 0, $w, $h)
      $bd = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, $fmt)
      for ($y = 0; $y -lt $h; $y++) {
        [Runtime.InteropServices.Marshal]::Copy($bytes, $y * $w * 4, [IntPtr]($bd.Scan0.ToInt64() + $y * $bd.Stride), $w * 4)
      }
      $bmp.UnlockBits($bd)
      $bmp.Save($f[4], [System.Drawing.Imaging.ImageFormat]::Gif)
      $bmp.Dispose()
    } else {
      $bmp = New-Object System.Drawing.Bitmap($f[1])
      $w = $bmp.Width; $h = $bmp.Height
      $rect = New-Object System.Drawing.Rectangle(0, 0, $w, $h)
      $bd = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, $fmt)
      $out = New-Object byte[] (8 + $w * $h * 4)
      [BitConverter]::GetBytes([int]$w).CopyTo($out, 0)
      [BitConverter]::GetBytes([int]$h).CopyTo($out, 4)
      for ($y = 0; $y -lt $h; $y++) {
        [Runtime.InteropServices.Marshal]::Copy([IntPtr]($bd.Scan0.ToInt64() + $y * $bd.Stride), $out, 8 + $y * $w * 4, $w * 4)
      }
      $bmp.UnlockBits($bd)
      $bmp.Dispose()
      [IO.File]::WriteAllBytes($f[2], $out)
    }
  } catch { Write-Output "FAIL $line : $_" }
}
'''


def gdip_run(work, jobs):
    """jobs: ('enc', w, h, raw, gif) / ('dec', gif, raw) -> output text (FAIL lines)."""
    script, lst = work / "gdip.ps1", work / "gdip_jobs.txt"
    script.write_text(PS_SCRIPT, encoding="utf-8")
    lst.write_text("\n".join("|".join(str(x) for x in j) for j in jobs) + "\n", encoding="utf-8")
    p = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script),
                        str(lst)], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1200)
    return p.stdout + p.stderr


def gdip_read(path):
    raw = path.read_bytes()
    w, h = struct.unpack_from("<II", raw)
    return premul_bgra(np.frombuffer(raw, np.uint8, offset=8).reshape(h, w, 4))


def ffmpeg_run(work, cmds):
    script = work / "ff.sh"
    script.write_bytes(("\n".join(cmds) + "\n").encode())
    p = subprocess.run(["wsl.exe", "-d", build.WSL_DISTRO, "--cd", str(ROOT), "-e", "sh", build.to_wsl_path(script)],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1200)
    return p.stdout + p.stderr


def ff(*args):
    return "timeout 120 ffmpeg -nostdin -v error -y " + " ".join(args)


def ext_prepare(work, rng):
    """Make GIFs with GDI+ and ffmpeg and decode them with their own decoders, and
    decode Python-written edge cases with both.  -> dict with lists of
    (name, gif bytes, reference image or None) and notes."""
    work.mkdir(parents=True, exist_ok=True)
    wp = build.to_wsl_path
    imgs = {"drawing": to_bgra(drawing_image(301, 187, 21, 16)), "photo": to_bgra(photo_image(320, 200, 8)),
            "ui": to_bgra(ui_image(320, 240, 4))}
    al = to_bgra(photo_image(160, 120, 9))
    yy, xx = np.mgrid[0:120, 0:160]
    al[..., 3] = np.where((xx // 20 + yy // 20) % 2 == 0, 255, 0)
    imgs["alpha"] = al
    gd_jobs, ff_cmds, own = [], [], []
    for n, a in imgs.items():
        raw = work / f"src_{n}.raw"
        raw.write_bytes(a.tobytes())
        h, w = a.shape[:2]
        gd_jobs.append(("enc", w, h, raw, work / f"gdip_{n}.gif"))
        gd_jobs.append(("dec", work / f"gdip_{n}.gif", work / f"gdip_{n}.ref"))
        pg = "palettegen=reserve_transparent=1" if n == "alpha" else "palettegen=reserve_transparent=0"
        ff_cmds.append(ff("-f rawvideo -pix_fmt bgra", f"-s {w}x{h}", f"-i {wp(raw)}",
                          f'-vf "split[a][b];[a]{pg}[p];[b][p]paletteuse"', wp(work / f"ff_{n}.gif")))
        ff_cmds.append(ff(f"-i {wp(work / f'ff_{n}.gif')}", "-frames:v 1 -f rawvideo -pix_fmt bgra",
                          wp(work / f"ff_{n}.ref")))
    for n, extra in (("anim", "testsrc=size=97x61:rate=10:duration=0.6"),
                     ("anim2", "mandelbrot=size=128x96:rate=10,trim=duration=0.6")):
        ff_cmds.append(ff(f"-f lavfi -i {extra}", '-vf "split[a][b];[a]palettegen[p];[b][p]paletteuse"',
                          wp(work / f"ff_{n}.gif")))
        ff_cmds.append(ff(f"-i {wp(work / f'ff_{n}.gif')}", "-frames:v 1 -f rawvideo -pix_fmt bgra",
                          wp(work / f"ff_{n}.ref")))
        gd_jobs.append(("dec", work / f"ff_{n}.gif", work / f"ffgd_{n}.ref"))
    # Python-written edge cases (screen = frame, colour tables present)
    edge = [(n, s) for n, s in dec_cases(random.Random(77))
            if not n.startswith(("no colour", "index past", "local 4", "global 2", "screen", "frame", "animated",
                                 "odd 1x", "odd 3", "odd 2", "GCE", "last GCE", "extensions", "garbage",
                                 "no trailer", "more pixels", "transparent 200"))
            and s["frames"][0]["idx"].size <= 60000]
    for n, spec in edge:
        tag = "".join(c if c.isalnum() else "_" for c in n)
        data = make_gif(spec, random.Random(5))
        exp = expect_spec(spec)
        (work / f"py_{tag}.gif").write_bytes(data)
        own.append((n, tag, data, exp))
        gd_jobs.append(("dec", work / f"py_{tag}.gif", work / f"pygd_{tag}.ref"))
        h, w = exp.shape[:2]
        ff_cmds.append(ff(f"-i {wp(work / f'py_{tag}.gif')}", "-frames:v 1 -f rawvideo -pix_fmt bgra",
                          wp(work / f"pyff_{tag}.ref")))
    log = gdip_run(work, gd_jobs) + ffmpeg_run(work, ff_cmds)
    files = []
    for n, a in imgs.items():
        for tool in ("gdip", "ff"):
            g, r = work / f"{tool}_{n}.gif", work / f"{tool}_{n}.ref"
            if not g.exists():
                continue
            if tool == "gdip":
                ref = gdip_read(r) if r.exists() else None
            else:
                ref = (premul_bgra(np.frombuffer(r.read_bytes(), np.uint8).reshape(a.shape))
                       if r.exists() else None)
            files.append((f"{tool} {n}", g.read_bytes(), ref, a))
    for n in ("anim", "anim2"):
        g, r, r2 = work / f"ff_{n}.gif", work / f"ff_{n}.ref", work / f"ffgd_{n}.ref"
        if g.exists():
            data = g.read_bytes()
            res, img, _ = gif_model(data)
            raw = r.read_bytes() if r.exists() else b""
            ref = (premul_bgra(np.frombuffer(raw, np.uint8).reshape(img.shape))
                   if res == 0 and len(raw) == img.size else None)
            files.append((f"ff {n}", data, ref, None))
            if r2.exists():
                files.append((f"ff {n} (GDI+ ref)", data, gdip_read(r2), None))
    return {"files": files, "own": own, "log": log, "work": work}


def suite_ext_dec(runner, prep, suite):
    """Our decoder vs GDI+ / ffmpeg decodes of their own files, and their decodes of Python-written
    edge cases vs the construction (the second part is target independent but cheap)."""
    files = [f for f in prep["files"] if f[2] is not None]
    recs = [Rec(OP_GIFDEC, 0, data, expect_result(0, ref), name) for name, data, ref, _ in files]
    crashes = []
    for r, res in zip(recs, runner.batch(recs, crashes)):
        msg = "crash" if res is None else r.check(*res)
        if msg:
            suite.fail(f"{r.name}: {msg}")
        else:
            suite.ok()
    for name, data, ref, src in files:
        if src is not None and ref is not None and "alpha" not in name:
            suite.notes.append(f"{name}: {len(data)} bytes, PSNR {psnr(ref, src):.2f} dB (their encoder)")
    work = prep["work"]
    for n in ("anim", "anim2"):
        if (work / f"ff_{n}.gif").exists() and not (work / f"ffgd_{n}.ref").exists():
            suite.notes.append(f"GDI+ cannot open ffmpeg's '{n}' GIF (no GDI+ reference, not counted)")
    missing = [n for n in ("gdip drawing", "ff drawing", "ff anim") if n not in [f[0] for f in files]]
    if missing:
        suite.fail(f"external files missing: {missing}; log: {prep['log'][:300]}")
    return suite


def ext_own_checks(prep, suite):
    """GDI+ / ffmpeg decodes of Python-written edge cases vs the construction (target independent).
    GDI+ does not support LZW minimum code size 1 at all (it fails or fills with index 0, also when
    the data is coded as for size 2); those files only produce notes.  ffmpeg, giflib and Wuffs
    (Chrome) agree with gif_decode there."""
    work = prep["work"]
    gdip_mcs1 = []
    for n, tag, data, exp in prep["own"]:
        for tool, path in (("GDI+", work / f"pygd_{tag}.ref"), ("ffmpeg", work / f"pyff_{tag}.ref")):
            if tool == "GDI+" and n.startswith("mcs1"):
                ok = path.exists() and np.array_equal(gdip_read(path), exp)
                gdip_mcs1.append(f"{n}: {'same' if ok else 'differs' if path.exists() else 'fails'}")
                continue
            if not path.exists():
                suite.fail(f"{tool} could not decode '{n}'")
                continue
            if tool == "GDI+":
                got = gdip_read(path)
            else:
                raw = path.read_bytes()
                if len(raw) != exp.size:
                    suite.fail(f"{tool} '{n}': {len(raw)} bytes, want {exp.size}")
                    continue
                got = premul_bgra(np.frombuffer(raw, np.uint8).reshape(exp.shape))
            if got.shape != exp.shape or not np.array_equal(got, exp):
                suite.fail(f"{tool} disagrees on '{n}': " + (img_diff(got, exp) if got.shape == exp.shape
                                                             else f"shape {got.shape}"))
            else:
                suite.ok()
    if gdip_mcs1:
        suite.notes.append("GDI+ on minimum code size 1 (unsupported by GDI+, not counted): "
                           + "; ".join(gdip_mcs1))


def ext_enc_checks(work, made_by_target, suite):
    """Our encoder's files: identical on every target, decoded by GDI+ and ffmpeg like gif_model."""
    work.mkdir(parents=True, exist_ok=True)
    targets = list(made_by_target)
    base = made_by_target[targets[0]]
    for t in targets[1:]:
        other = {n: g for n, g, _, _ in made_by_target[t]}
        for n, g, _, _ in base:
            if other.get(n) != g:
                suite.fail(f"encoder output of '{n}' differs between {targets[0]} and {t}")
            else:
                suite.ok()
    jobs, cmds, sel = [], [], []
    for n, g, img, _ in base:
        if img.size > 4 * 1200 * 700:
            continue
        tag = "".join(c if c.isalnum() else "_" for c in n)
        p = work / f"ours_{tag}.gif"
        p.write_bytes(g)
        jobs.append(("dec", p, work / f"oursgd_{tag}.ref"))
        cmds.append(ff(f"-i {build.to_wsl_path(p)}", "-frames:v 1 -f rawvideo -pix_fmt bgra",
                       build.to_wsl_path(work / f"oursff_{tag}.ref")))
        sel.append((n, tag, img))
    gdip_run(work, jobs)
    ffmpeg_run(work, cmds)
    for n, tag, img in sel:
        for tool, path in (("GDI+", work / f"oursgd_{tag}.ref"), ("ffmpeg", work / f"oursff_{tag}.ref")):
            if not path.exists():
                suite.fail(f"{tool} could not decode our '{n}'")
                continue
            if tool == "GDI+":
                got = gdip_read(path)
            else:
                raw = path.read_bytes()
                if len(raw) != img.size:
                    suite.fail(f"{tool} on our '{n}': {len(raw)} bytes, want {img.size}")
                    continue
                got = premul_bgra(np.frombuffer(raw, np.uint8).reshape(img.shape))
            if got.shape != img.shape or not np.array_equal(got, img):
                suite.fail(f"{tool} decodes our '{n}' differently: "
                           + (img_diff(got, img) if got.shape == img.shape else f"shape {got.shape}"))
            else:
                suite.ok()


# ---------------------------------------------------------------- bench

def run_bench(runner, work, suite, with_ffmpeg):
    photo = to_bgra(photo_image(1152, 648, 1))
    draw = to_bgra(drawing_image(1152, 648, 4, 24))
    big = to_bgra(photo_image(1920, 1080, 2))
    bigd = to_bgra(drawing_image(1920, 1080, 6, 24))
    crashes = []
    recs = [Rec(OP_GIFENC_T, 5, surf_bytes(0, photo), None, "enc photo"),
            Rec(OP_GIFENC_T, 10, surf_bytes(0, draw), None, "enc drawing"),
            Rec(OP_GIFENC, 0, surf_bytes(0, big), None, "make 1920x1080 photo"),
            Rec(OP_GIFENC, 0, surf_bytes(0, bigd), None, "make 1920x1080 drawing"),
            Rec(OP_GIFENC, 0, surf_bytes(0, photo), None, "photo file")]
    res = runner.batch(recs, crashes)
    if any(r is None or r[0] != 0 for r in res):
        suite.fail(f"bench encode failed: {[r[0] if r else None for r in res]} {crashes[:2]}")
        return
    us_p, len_p = struct.unpack_from("<II", res[0][1])
    us_d, len_d = struct.unpack_from("<II", res[1][1])
    gif_big, gif_bigd, gif_photo = res[2][1], res[3][1], res[4][1]
    recs = [Rec(OP_GIFDEC_T, 10, gif_big, None, "dec 1920x1080 photo"),
            Rec(OP_GIFDEC_T, 10, gif_bigd, None, "dec 1920x1080 drawing")]
    res = runner.batch(recs, crashes)
    if any(r is None or r[0] != 0 for r in res):
        suite.fail(f"bench decode failed {crashes[:2]}")
        return
    us_bp = struct.unpack_from("<I", res[0][1])[0]
    us_bd = struct.unpack_from("<I", res[1][1])[0]
    _, img, _ = gif_model(gif_photo)
    p_ours = psnr(img, photo)
    suite.notes.append(f"encode 1152x648 photo   : {us_p / 1000:7.1f} ms  {len_p:8d} bytes  PSNR {p_ours:.2f} dB"
                       f"  (target < 80 ms)")
    suite.notes.append(f"encode 1152x648 drawing : {us_d / 1000:7.1f} ms  {len_d:8d} bytes  (target < 20 ms)")
    suite.notes.append(f"decode 1920x1080 photo  : {us_bp / 1000:7.1f} ms  {len(gif_big):8d} bytes  (target < 30 ms)")
    suite.notes.append(f"decode 1920x1080 drawing: {us_bd / 1000:7.1f} ms  {len(gif_bigd):8d} bytes")
    for nm, v, lim in (("photo encode", us_p, 80000), ("drawing encode", us_d, 20000),
                       ("1080p decode", us_bp, 30000)):
        if v > lim:
            suite.fail(f"{nm} {v / 1000:.1f} ms over the {lim / 1000:.0f} ms target")
        else:
            suite.ok()
    if with_ffmpeg:
        work.mkdir(parents=True, exist_ok=True)
        raw = work / "bench_photo.raw"
        raw.write_bytes(photo.tobytes())
        wp = build.to_wsl_path
        cmds = []
        for tag, pu in (("sierra", "paletteuse"), ("fs", "paletteuse=dither=floyd_steinberg"),
                        ("none", "paletteuse=dither=none")):
            cmds.append(ff("-f rawvideo -pix_fmt bgra -s 1152x648", f"-i {wp(raw)}",
                           f'-vf "split[a][b];[a]palettegen=reserve_transparent=0[p];[b][p]{pu}"',
                           wp(work / f"bench_ff_{tag}.gif")))
        ffmpeg_run(work, cmds)
        for tag in ("sierra", "fs", "none"):
            g = work / f"bench_ff_{tag}.gif"
            if g.exists():
                data = g.read_bytes()
                r, img, _ = gif_model(data)
                if r == 0:
                    suite.notes.append(f"ffmpeg palettegen + paletteuse ({tag:6s}): {len(data):8d} bytes  "
                                       f"PSNR {psnr(img, photo):.2f} dB")


# ---------------------------------------------------------------- main


# ---------------------------------------------------------------- CA package: options and metadata

OP_QUANTEX, OP_GIFENCQ, OP_GIFDECFL = 45, 46, 47


def _chk(suite, cond, msg):
    if cond:
        suite.ok()
    else:
        suite.fail(msg)
OP_DECEX, OP_ENCODE, OP_METAFULL = 240, 252, 261
JDF_STRAIGHT, FMT_ARGB = 1, 3
IQF_KEEPALPHA, IQF_WHITE = 1 << 20, 1 << 21


def qex_bytes(cfg, fmt, arr):
    return struct.pack("<I", cfg) + surf_bytes(fmt, arr)


def seen_straight(fmt, arr):
    """straight BGRA the quantizer works on (before thresholding)."""
    if fmt == 2:
        g = arr.astype(np.uint8)
        return np.stack([g, g, g, np.full_like(g, 255)], -1)
    if fmt == 0:
        out = arr.copy()
        out[..., 3] = 255
        return out
    if fmt == FMT_ARGB:
        return arr.copy()
    st = unpremul_ref(arr)
    return np.stack([st[..., 2], st[..., 1], st[..., 0], st[..., 3]], -1)


def thresholded(fmt, arr, thr):
    s = seen_straight(fmt, arr)
    out = s.copy()
    out[..., 3] = 255
    out[s[..., 3] < thr] = 0
    return out


def suite_quantex(rng):
    """img_quantize_ex (op 45): dither levels, thresholds, IQF_WHITE, IQF_KEEPALPHA, FMT_ARGB;
    level 0 / 8 identical to img_quantize (op 42) dither off / on."""
    recs = []
    nr = np.random.default_rng(rng.randrange(1 << 30))
    ph = photo_image(96, 64, 3)
    base = to_bgra(ph)
    yy, xx = np.mgrid[0:64, 0:96]
    base[..., 3] = np.clip(xx * 3, 0, 255).astype(np.uint8)
    base[0, 0] = (1, 2, 3, 0)                                  # colour at alpha 0 (straight input)
    pm = premultiply(base)
    n = 96 * 64

    def exact_of(img):
        flat = img.reshape(-1, 4).view(np.uint32).ravel()
        order, seen = [], {}
        for v in flat:
            if v not in seen:
                seen[v] = len(order)
                order.append(v)
        return order, np.array([seen[v] for v in flat], np.int64), seen.get(np.uint32(0), -1)

    def chk_exact(img, maxc=256):
        order, want_idx, want_tr = exact_of(img)

        def chk(res, out):
            if res != 0:
                return f"record {res}"
            cnt, tr, pal, idx = quant_parse(out, img.shape[0] * img.shape[1])
            if len(order) > maxc:
                return None if cnt == 0 else f"count {cnt} (expected 0: too many colours)"
            if cnt != len(order) or tr != want_tr:
                return f"count {cnt} / {len(order)}, trans {tr} / {want_tr}"
            if not np.array_equal(pal[:cnt], np.array(order, np.uint32)) or not np.array_equal(idx, want_idx):
                return "palette / indices differ"
            return None
        return chk

    def chk_valid(img_thr, maxc, min_psnr):
        flat = img_thr.reshape(-1, 4)
        tm = flat[:, 3] == 0

        def chk(res, out):
            if res != 0:
                return f"record {res}"
            cnt, tr, pal, idx = quant_parse(out, len(flat))
            if not 1 <= cnt <= maxc or np.any(idx >= cnt):
                return f"count {cnt} / indices"
            pb = pal_bgra(pal)
            if tm.any() and (tr < 0 or not np.array_equal(idx == tr, tm)):
                return "transparent mapping"
            if not tm.any() and tr != -1:
                return f"transparent index {tr}"
            got = pb[idx].astype(np.int64)
            if np.any(got[~tm, 3] != 255):
                return "opaque pixel on a transparent entry"
            p = psnr(got[~tm], flat[~tm].astype(np.int64)) if (~tm).any() else 99
            return None if p >= min_psnr else f"PSNR {p:.1f} < {min_psnr}"
        return chk

    # thresholds on straight / premultiplied input, exact palette of a drawing
    d = to_bgra(drawing_image(40, 30, 8, 12))
    d[..., 3] = (np.arange(40 * 30).reshape(30, 40) * 7 % 256).astype(np.uint8)
    for fmt, arr in ((FMT_ARGB, d), (1, premultiply(d))):
        for thr in (0, 1, 64, 128, 200, 255, 256):
            img = thresholded(fmt, arr, thr)
            recs.append(Rec(OP_QUANTEX, 256, qex_bytes(8 | thr << 8, fmt, arr), chk_exact(img) if ncolours(img) <= 256
                            else chk_valid(img, 256, 20), f"qex thr {thr} fmt {fmt}"))
    # IQF_WHITE: composited over white, never transparent
    for fmt, arr in ((FMT_ARGB, d), (1, premultiply(d))):
        s = seen_straight(fmt, arr).astype(np.int64)
        a = s[..., 3:4]
        w = np.concatenate([(s[..., :3] * a + 127) // 255 + 255 - a, np.full_like(a, 255)], -1).astype(np.uint8)
        recs.append(Rec(OP_QUANTEX, 256, qex_bytes(8 | 128 << 8 | IQF_WHITE, fmt, arr),
                        chk_exact(w) if ncolours(w) <= 256 else chk_valid(w, 256, 20), f"qex white fmt {fmt}"))
    # IQF_KEEPALPHA: exact with per-entry alpha; too many colours -> 0
    few = np.array([(10, 20, 30, 255), (1, 2, 3, 128), (9, 9, 9, 0), (200, 100, 50, 7)], np.uint8)
    img = few[nr.integers(0, 4, (13, 11))]
    img[0, 0] = (5, 6, 7, 0)                                   # another alpha-0 colour: counts as 0
    norm = img.copy()
    norm[norm[..., 3] == 0] = 0
    recs.append(Rec(OP_QUANTEX, 256, qex_bytes(IQF_KEEPALPHA, FMT_ARGB, img), chk_exact(norm), "qex keepalpha"))
    recs.append(Rec(OP_QUANTEX, 3, qex_bytes(IQF_KEEPALPHA, FMT_ARGB, img), chk_exact(norm, 3), "qex keepalpha > max"))
    recs.append(Rec(OP_QUANTEX, 256, qex_bytes(IQF_KEEPALPHA, FMT_ARGB, base), chk_exact(
        np.where(base[..., 3:4] == 0, 0, base).astype(np.uint8)), "qex keepalpha photo"))
    # dither levels: valid, PSNR bounds; level 0 / 8 == img_quantize dither 0 / 1 (bit-identical)
    opq = to_bgra(ph)
    for lv in range(9):
        recs.append(Rec(OP_QUANTEX, 64, qex_bytes(lv | 128 << 8, 0, opq), chk_valid(opq, 64, 20), f"qex level {lv}"))
        recs.append(Rec(OP_QUANTEX, 256, qex_bytes(lv | 128 << 8, 1, pm),
                        chk_valid(thresholded(1, pm, 128), 256, 20), f"qex level {lv} alpha"))
    return recs


def quantex_compat(runner, rng, suite):
    """level 0 / 8 with threshold 128 must equal img_quantize dither 0 / 1 exactly; levels
    in between must differ from both ends on a photo (the strength really changes)."""
    ph = to_bgra(photo_image(120, 80, 9))
    pm = premultiply(np.concatenate([ph[..., :3], (np.arange(120 * 80).reshape(80, 120) % 256)[..., None]],
                                    -1).astype(np.uint8))
    recs, pairs = [], []
    for fmt, arr in ((0, ph), (1, pm)):
        for maxc in (16, 256):
            for dth, lv in ((0, 0), (1, 8)):
                pairs.append((len(recs), f"fmt {fmt} max {maxc} dither {dth}"))
                recs.append(Rec(OP_QUANT, maxc | dth << 16, surf_bytes(fmt, arr), None, "q"))
                recs.append(Rec(OP_QUANTEX, maxc, qex_bytes(lv | 128 << 8, fmt, arr), None, "qex"))
            recs.append(Rec(OP_QUANTEX, maxc, qex_bytes(4 | 128 << 8, fmt, arr), None, "qex4"))
    res = runner.batch(recs, [])
    for i, nm in pairs:
        a, b = res[i], res[i + 1]
        _chk(suite, a is not None and b is not None and a == b, f"img_quantize_ex != img_quantize: {nm}")
    for k in range(0, len(res), 5):
        l0, l8, l4 = res[k + 1], res[k + 3], res[k + 4]
        _chk(suite, l4 is not None and l4 != l0 and l4 != l8, f"level 4 equals level 0 or 8 (group {k // 5})")


def gif_icc_file(icc, blocks=255):
    img = np.zeros((3, 4), np.uint8)
    body = subblocks(icc, blocks)
    app = bytes((0x21, 0xFF, 11)) + b"ICCRGBG1012" + body
    codes = lzw_codes(img.ravel().tolist(), 2)
    lzw = pack_codes(codes, 2)
    return (b"GIF89a" + struct.pack("<HHBBB", 4, 3, 0x80, 0, 0) + bytes(6) + app +
            b"\x2c" + struct.pack("<HHHHB", 0, 0, 4, 3, 0) + bytes((2,)) + subblocks(lzw) + b"\x3b")


def suite_camisc(rng):
    """gif_decode_fl straight output, gif_encode_q, img_encode options, IMGMETA (frames, loops,
    delay, ICCRGBG1 profile)."""
    from png_check import meta_full
    recs = []
    nr = np.random.default_rng(rng.randrange(1 << 30))
    # straight decode = same bytes, FMT_ARGB
    for name, spec in dec_cases(rng)[:40]:
        data = make_gif(spec, rng)
        res, img, _ = gif_model(data)

        def chk(r, out, res=res, img=img):
            if r != res:
                return f"result {r} != {res}"
            if r != 0:
                return None
            w, h, f = struct.unpack_from("<III", out)
            if f & 0xFF != FMT_ARGB:
                return f"fmt {f & 0xFF}"
            got = np.frombuffer(out, np.uint8, w * h * 4, 12).reshape(h, w, 4)
            return None if np.array_equal(got, img) else "pixels differ"
        recs.append(Rec(OP_GIFDECFL, JDF_STRAIGHT, data, chk, "fl straight " + name))
    # gif_encode_q: threshold and dither level, ARGB input
    d = to_bgra(drawing_image(50, 40, 3, 20))
    d[..., 3] = (np.arange(2000).reshape(40, 50) % 256).astype(np.uint8)
    for thr in (0, 1, 100, 255):
        for lv in (0, 5, 8):
            want = thresholded(FMT_ARGB, d, thr)

            def chk(r, out, want=want):
                if r != 0:
                    return f"error {r}"
                res, img, _ = gif_model(bytes(out))
                if res != 0:
                    return f"model {res}"
                if not np.array_equal(img[..., 3] == 0, want[..., 3] == 0):
                    return "transparency differs from the threshold"
                return None if ncolours(want) > 256 or np.array_equal(img, want) else "not exact"
            recs.append(Rec(OP_GIFENCQ, lv | thr << 8, surf_bytes(FMT_ARGB, d), chk, f"encq thr {thr} level {lv}"))
            opt = struct.pack("<12IQQ", 4, 0, 0, 0, 0, lv + 1, thr + 1, 0, 0, 0, 0, 0, 0, 0)
            recs.append(Rec(OP_ENCODE, 0, opt + surf_bytes(FMT_ARGB, d), chk, f"img_encode thr {thr} level {lv}"))
    # IMGMETA: frames, loops, delay, ICC
    icc = bytes(nr.integers(0, 256, 1000, dtype=np.uint8))
    for blocks in (255, 17, 1):
        data = gif_icc_file(icc, blocks)
        recs.append(Rec(OP_METAFULL, 2, data, lambda r, o: None if r == 0 and meta_full(o)["icc"] == icc else
                        "ICC", f"ICCRGBG1 blocks {blocks}"))
    recs.append(Rec(OP_METAFULL, 0, gif_icc_file(icc), lambda r, o: None if r == 0 and not meta_full(o)["icc"]
                    else "ICC without IMGD_ICC", "ICCRGBG1 not asked"))
    cut = gif_icc_file(icc)[:400]
    recs.append(Rec(OP_METAFULL, 2 | 32, cut, lambda r, o: None if r == 0 and not meta_full(o)["icc"]
                    else f"cut ICC: {r}", "ICCRGBG1 cut"))
    for name, spec in dec_cases(rng):
        if "anim" not in name:
            continue
        data = make_gif(spec, rng)
        nfr = data.count(b"\x2c")                          # a rough upper bound only
        recs.append(Rec(OP_METAFULL, 32, data, lambda r, o, nfr=nfr: None if r == 0 and 1 <= meta_full(o)["frames"]
                        <= nfr else "frames", "meta " + name))
    netscape = (bytes((0x21, 0xFF, 11)) + b"NETSCAPE2.0" + bytes((3, 1)) + struct.pack("<H", 5) + b"\0")
    two = gif_icc_file(b"")
    body = two[13 + 6:-1]
    anim = two[:13 + 6] + netscape + gce(None) + body + body + b"\x3b"
    recs.append(Rec(OP_METAFULL, 32, anim, lambda r, o: None if r == 0 and meta_full(o)["frames"] == 2 and
                    meta_full(o)["loops"] == 5 else f"frames / loops {meta_full(o)['frames'], meta_full(o)['loops']}",
                    "two frames, 5 loops"))
    return recs



def classic_identity(runner, rng, suite):
    """gif_encode (op 41), img_quantize (op 42) and gif_decode (op 40) of this build == the C0
    reference binary (build/ca/ref/codectest_c0.exe, the state before the CA package)."""
    import imgtest_common as it
    ref = ROOT / "build" / "ca" / "ref" / "codectest_c0.exe"
    if not ref.exists():
        suite.notes.append("classic identity: reference binary missing, skipped")
        return
    rr = it.Runner("win", runner.exe.parent.parent, work_name="gif_ref")
    rr.exe = ref
    recs = []
    for name, fmt, arr, _ in enc_surfaces(rng):
        if arr.shape[0] * arr.shape[1] > 400000:
            continue
        recs.append(Rec(OP_GIFENC, 0, surf_bytes(fmt, arr), None, "enc " + name))
        for maxc, dth in ((256, 1), (256, 0), (16, 1)):
            recs.append(Rec(OP_QUANT, maxc | dth << 16, surf_bytes(fmt, arr), None, f"quant {maxc} {dth} {name}"))
    new = runner.batch(recs, [])
    old = rr.batch(recs)
    for r, a, b in zip(recs, new, old):
        if a is None or b is None or a != b:
            suite.fail(f"differs: {r.name}")
        else:
            suite.ok()
    dec = [Rec(OP_GIFDEC, 0, bytes(a[1]), None, "dec " + r.name) for r, a in zip(recs, new)
           if r.op == OP_GIFENC and a and a[0] == 0]
    for r, a, b in zip(dec, runner.batch(dec, []), rr.batch(dec)):
        if a is None or b is None or a != b:
            suite.fail(f"decode differs: {r.name}")
        else:
            suite.ok()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build", help="build directory (default: build)")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--only", default="", help="comma list of suites (model,dec,fuzz,quant,quantex,camisc,classic,enc,ext,bench)")
    ap.add_argument("--fuzz", type=int, default=4000, help="fuzz cases")
    ap.add_argument("--no-ext", action="store_true", help="skip GDI+ / ffmpeg")
    ap.add_argument("--no-bench", action="store_true")
    ap.add_argument("--seed", type=int, default=4321)
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        build_targets(targets, out)
    all_ok = True

    def report(tag, s, dt):
        nonlocal all_ok
        status = "PASS" if s.failed == 0 else "FAIL"
        print(f"[{tag}] {status} {s.name:6s} {s.passed:6d} passed {s.failed:5d} failed  ({dt:.1f}s)")
        for n in s.notes:
            print("        " + n)
        for m in s.msgs:
            print("        ! " + m)
        all_ok &= s.failed == 0

    if want("model"):
        t0 = time.time()
        report("py", suite_model(random.Random(a.seed)), time.time() - t0)
    work = out / "gif-work"
    work.mkdir(parents=True, exist_ok=True)
    use_ext = want("ext") and not a.no_ext
    prep = None
    if use_ext:
        t0 = time.time()
        prep = ext_prepare(work / "ext", random.Random(a.seed + 7))
        s = Suite("ext-py")
        ext_own_checks(prep, s)
        report("host", s, time.time() - t0)
    made_by_target = {}
    for target in targets:
        runner = Runner(target, out)
        runner.work = out / target / "gif"
        runner.work.mkdir(parents=True, exist_ok=True)
        print(f"==== {target}")
        for name, fn in (("dec", suite_dec), ("fuzz", lambda r: suite_fuzz(r, a.fuzz)), ("quant", suite_quant),
                         ("quantex", suite_quantex), ("camisc", suite_camisc)):
            if want(name):
                s = Suite(name)
                dt = run_suite(runner, s, fn(random.Random(a.seed + len(name))))
                report(target, s, dt)
        if want("classic") and target == "win":
            s = Suite("classic")
            t0 = time.time()
            classic_identity(runner, random.Random(a.seed + 12), s)
            report(target, s, time.time() - t0)
        if want("quantex"):
            s = Suite("qcompat")
            t0 = time.time()
            quantex_compat(runner, random.Random(a.seed + 11), s)
            report(target, s, time.time() - t0)
        if want("enc") or use_ext:
            s = Suite("enc")
            t0 = time.time()
            made_by_target[target] = run_enc(runner, random.Random(a.seed + 3), s)
            report(target, s, time.time() - t0)
        if use_ext:
            s = Suite("ext")
            t0 = time.time()
            suite_ext_dec(runner, prep, s)
            report(target, s, time.time() - t0)
        if want("bench") and not a.no_bench:
            s = Suite("bench")
            t0 = time.time()
            run_bench(runner, work, s, not a.no_ext)
            report(target, s, time.time() - t0)
    if use_ext and made_by_target:
        s = Suite("ext-enc")
        t0 = time.time()
        ext_enc_checks(work / "ours", made_by_target, s)
        report("host", s, time.time() - t0)
    print("gif_check:", "ALL PASSED" if all_ok else "FAILURES")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
