#!/usr/bin/env python3
"""BMP checks for the CA core-codec package (img/bmp.s, img/imgcore.s, img/quantize.s).

Runs apps/codectest batch ops (240 img_decode_ex, 252 img_encode, 261 whole IMGMETA from
png_ops.s, 280 / 281 from apps/codectest/src/bmp_ops.s) and compares with Python references:

  straight  32-bit BI_BITFIELDS / BI_ALPHABITFIELDS and 16-bit 4444 files with alpha decoded
            with JDF_STRAIGHT (op 280) / IMGD_STRAIGHT (op 240): the file's straight values
            (colour kept at alpha 0); premultiplied output == premul(straight)
  rle       BI_RLE8 / BI_RLE4 streams from a Python writer (encoded runs incl. RLE4 colour
            pairs, absolute runs with word padding, end of line, end of bitmap, delta, runs
            past the right edge, deltas past the image, data that ends inside a run, no
            end of bitmap, top-down) against a Python model of the decoder (skipped pixels
            transparent); full images (no skipped pixels) also against ffmpeg's decoder
  embed     BI_JPEG / BI_PNG: equal to decoding the embedded file directly (biSizeImage 0 /
            exact / too large), straight flag passed on
  unsup     OS/2 2.x Huffman 1D and RLE24, CMYK compressions -> ERR_UNSUPPORTED
  meta      V5 embedded ICC profile (PROFILE_EMBEDDED), LCS_sRGB, pixels per metre -> dpi
  enc       bmp_encode_cfg (op 281) / img_encode (op 252): depth auto / 32 / 24 / 8 (+ dither),
            XRGB / PARGB / ARGB / A8 input, pixels per metre; read back with the Python reader
            and WIC; classic bmp_encode byte-identical to the C0 reference (build/ca/ref)
  fuzz      mutated RLE / embedded / V5 files through op 240 and op 280 (guard pages)

  python tools/bmp_check.py --target both --out build/ca
"""

import random
import struct
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
from codec_check import bmp_make, mask_scale  # noqa: E402
from png_check import (expect_surface, fake_icc, imgopt, meta_full, over_white, src_image,  # noqa: E402
                       as_input, ppm_dpi)

C = it.C
OP_METAFULL, OP_BMPDECFL, OP_BMPENCCFG = 261, 280, 281
SUITES = "straight,rle,embed,unsup,meta,enc,fuzz"


# ================================================================ references

def masked_straight(px, masks):
    px = px.astype(np.int64)
    ch = []
    for k in range(4):
        s = mask_scale(px, masks[k] if k < len(masks) else 0)
        ch.append(np.full(px.shape, 255 if k == 3 else 0, np.int64) if s is None else s)
    return np.stack([ch[2], ch[1], ch[0], ch[3]], -1).astype(np.uint8)


def bmp_file(hsz, w, h, bpp, comp, data, pal=b"", clr_used=0, extra_hdr=b"", tail=b"", size_image=None,
             ppm=(2835, 2835)):
    """Generic writer: header (hsz bytes, extra_hdr overlays bytes 40..), palette, data, tail."""
    hdr = struct.pack("<IiiHHIIiiII", hsz, w, h, 1, bpp, comp, len(data) if size_image is None else size_image,
                      ppm[0], ppm[1], clr_used, 0)
    hdr = (hdr + extra_hdr + b"\0" * hsz)[:hsz]
    off = 14 + hsz + len(pal)
    body = hdr + pal + data + tail
    return b"BM" + struct.pack("<IHHI", 14 + len(body), 0, 0, off) + body


def pal_bytes(pal):
    return b"".join(bytes([b, g, r, 0]) for (r, g, b) in pal)


def lut_of(pal):
    lut = np.zeros((256, 4), np.uint8)
    lut[:, 3] = 255
    for i, (r, g, b) in enumerate(pal[:256]):
        lut[i] = (b, g, r, 255)
    return lut


def rle_model(s, w, h, bpp, lut, topdown=False):
    """Python model of bmp_rle: -> (h, w, 4) BGRA (skipped pixels 0)."""
    out = np.zeros((h, w, 4), np.uint8)
    x = y = p = 0
    n_ = len(s)

    def put(xx, yy, idx):
        out[yy if topdown else h - 1 - yy, xx] = lut[idx]
    while y < h and p + 2 <= n_:
        n, c = s[p], s[p + 1]
        p += 2
        if n:
            for i in range(n):
                if x >= w:
                    break
                idx = c if bpp == 8 else (c >> 4 if i % 2 == 0 else c & 15)
                put(x, y, idx)
                x += 1
        elif c == 0:
            x, y = 0, y + 1
        elif c == 1:
            break
        elif c == 2:
            if p + 2 > n_:
                break
            x = min(x + s[p], w)
            y += s[p + 1]
            p += 2
        else:
            m = c
            nb = m if bpp == 8 else (m + 1) // 2
            if n_ - p < nb:
                nb = n_ - p
                m = nb if bpp == 8 else 2 * nb
            for i in range(m):
                if x >= w:
                    break
                b = s[p + (i if bpp == 8 else i // 2)]
                idx = b if bpp == 8 else (b >> 4 if i % 2 == 0 else b & 15)
                put(x, y, idx)
                x += 1
            p += min(nb + (nb & 1), n_ - p)
    return out


def rle_from_image(idx, bpp):
    """Valid RLE stream for an index image (bottom-up rows), no skipped pixels: encoded runs
    for repeats, absolute runs (>= 3 pixels) otherwise, EOL per row, EOB."""
    h, w = idx.shape
    out = bytearray()
    for y in range(h - 1, -1, -1):
        row = [int(v) for v in idx[y]]
        x = 0
        while x < w:
            run = 1
            while x + run < w and run < 255 and row[x + run] == row[x]:
                run += 1
            if run >= 2 or w - x < 3:
                v = row[x] if bpp == 8 else (row[x] << 4) | row[x]
                out += bytes([run, v])
                x += run
                continue
            # absolute run up to the next repeat
            e = x
            while e < w and e - x < 255 and not (e + 1 < w and row[e + 1] == row[e]):
                e += 1
            m = e - x
            if m < 3:
                v = row[x] if bpp == 8 else (row[x] << 4) | row[x]
                out += bytes([1, v])
                x += 1
                continue
            vals = row[x:e]
            if bpp == 8:
                b = bytes(vals)
            else:
                vv = vals + [0] * (m & 1)
                b = bytes((vv[i] << 4) | vv[i + 1] for i in range(0, len(vv), 2))
            out += bytes([0, m]) + b + (b"\0" if len(b) & 1 else b"")
            x = e
        out += b"\0\0"
    out += b"\0\1"
    return bytes(out)


def rle_random(rng, w, h, bpp):
    """Random command stream exercising every escape (not necessarily covering all pixels)."""
    out = bytearray()
    ncol = 256 if bpp == 8 else 16
    for _ in range(rng.randrange(1, 12 + w * h // 4)):
        k = rng.randrange(10)
        if k < 4:
            out += bytes([rng.randrange(1, 256 if rng.random() < 0.2 else min(256, w + 3)), rng.randrange(256)])
        elif k < 7:
            m = rng.randrange(3, min(255, w + 6) + 1)
            nb = m if bpp == 8 else (m + 1) // 2
            b = bytes(rng.randrange(256) if bpp == 8 else rng.randrange(256) for _ in range(nb))
            out += bytes([0, m]) + b + (b"\0" if nb & 1 else b"")
        elif k == 7:
            out += b"\0\0"
        elif k == 8:
            out += bytes([0, 2, rng.randrange(0, min(256, w + 2)), rng.randrange(0, 3)])
        else:
            out += bytes([rng.randrange(1, 5), rng.randrange(ncol)])
    if rng.random() < 0.7:
        out += b"\0\1"
    return bytes(out)


# ================================================================ suites

def suite_straight(rng):
    recs = []
    r = np.random.default_rng(rng.randrange(1 << 30))
    for (w, h) in ((1, 1), (5, 3), (17, 9), (64, 5)):
        for td in (False, True):
            px = r.integers(0, 1 << 32, (h, w), dtype=np.uint64)
            px[0, 0] = 0x00AABBCC                           # colour at alpha 0
            for hsz, masks, comp in ((108, [0xFF0000, 0xFF00, 0xFF, 0xFF000000], 3),
                                     (124, [0xFF0000, 0xFF00, 0xFF, 0xFF000000], 3),
                                     (56, [0xFF, 0xFF00, 0xFF0000, 0xFF000000], 3),
                                     (40, [0xFF000000, 0xFF0000, 0xFF00, 0xFF], 6),
                                     (40, [0xFFC00000, 0x3FF000, 0xFFC, 0x3], 6)):
                data = bmp_make(px, 32, hsz=hsz, comp=comp, masks=masks, topdown=td)
                st = masked_straight(px, masks)
                nm = f"32 hsz{hsz} comp{comp} {w}x{h} td{td}"
                recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, data, expect_surface(st, C.FMT_ARGB), "fl " + nm))
                recs.append(it.Rec(OP_BMPDECFL, 0, data, expect_surface(it.premul(st), C.FMT_PARGB), "fl pm " + nm))
                recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_STRAIGHT),
                                   it.expect_decex(st, fmt=C.FMT_ARGB), "decex " + nm))
            p16 = r.integers(0, 1 << 16, (h, w))
            m4444 = [0xF00, 0xF0, 0xF, 0xF000]
            data = bmp_make(p16, 16, comp=6, masks=m4444, topdown=td)
            st = masked_straight(p16, m4444)
            recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, data, expect_surface(st, C.FMT_ARGB), f"16 4444 {w}x{h}"))
            # opaque formats: straight == premultiplied bytes, fmt ARGB
            bgr = r.integers(0, 256, (h, w, 3))
            data = bmp_make(bgr, 24, topdown=td)
            st = np.concatenate([bgr, np.full((h, w, 1), 255)], -1).astype(np.uint8)
            recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, data, expect_surface(st, C.FMT_ARGB), f"24 {w}x{h} td{td}"))
    return recs


def suite_rle(rng, ext_dir, use_ext):
    recs = []
    ffjobs = []
    for bpp, comp in ((8, 1), (4, 2)):
        ncol = 1 << bpp
        for (w, h) in ((1, 1), (2, 3), (5, 4), (13, 7), (40, 9), (255, 2), (300, 3)):
            for td in (False, True):
                npal = rng.choice((ncol, max(1, ncol // 2)))
                pal = [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(npal)]
                lut = lut_of(pal)
                for k in range(4):
                    s = rle_random(rng, w, h, bpp)
                    data = bmp_file(40, w, -h if td else h, bpp, comp, s, pal_bytes(pal), npal)
                    exp = rle_model(s, w, h, bpp, lut, td)
                    nm = f"rle{bpp} random {w}x{h} td{td} #{k}"
                    recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, data, expect_surface(exp, C.FMT_ARGB), nm))
                    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data), it.expect_decex(it.premul(exp)),
                                       nm + " decex"))
                    # truncated inside the stream
                    cut = bmp_file(40, w, h, bpp, comp, s, pal_bytes(pal), npal)[:-max(1, len(s) // 3)]
                    exp2 = rle_model(s[:len(s) - max(1, len(s) // 3)], w, h, bpp, lut)
                    recs.append(it.Rec(OP_BMPDECFL, 0, cut, expect_surface(exp2, C.FMT_PARGB), nm + " cut"))
                # full image, no gaps: also for ffmpeg
                r = np.random.default_rng(rng.randrange(1 << 30))
                idx = r.integers(0, npal, (h, w))
                idx[:, : w // 2] = idx[:, :1]                   # some runs
                s = rle_from_image(idx, bpp)
                data = bmp_file(40, w, h, bpp, comp, s, pal_bytes(pal), npal)
                exp = rle_model(s, w, h, bpp, lut)
                full = lut[idx]
                recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, data, expect_surface(exp, C.FMT_ARGB),
                                   f"rle{bpp} image {w}x{h}"))
                if not np.array_equal(exp, full):
                    recs.append(it.Rec(OP_BMPDECFL, 0, b"", lambda r_, o_: "model / writer disagree",
                                       f"rle{bpp} writer {w}x{h}"))
                if use_ext and not td:
                    f = ext_dir / f"rle{bpp}_{w}x{h}.bmp"
                    f.write_bytes(data)
                    ffjobs.append((f, full, f"rle{bpp} image {w}x{h} vs ffmpeg"))
    # hand-written edge cases (RLE8, 4 x 3, palette i -> (i, i, i))
    pal = [(i, i, i) for i in range(256)]
    lut = lut_of(pal)
    cases = {
        "delta past image": bytes([2, 9, 0, 2, 200, 200, 2, 7]),
        "delta then run": bytes([1, 5, 0, 2, 2, 1, 2, 6, 0, 1]),
        "run past edge": bytes([200, 3, 2, 4, 0, 0, 3, 5]),
        "abs past edge": bytes([0, 6, 1, 2, 3, 4, 5, 6, 1, 9]),
        "abs odd pad": bytes([0, 3, 7, 8, 9, 0, 2, 1, 0, 1]),
        "eol eol eob": bytes([0, 0, 0, 0, 1, 4, 0, 1]),
        "no eob": bytes([4, 1, 0, 0, 4, 2, 0, 0, 4, 3]),
        "abs cut": bytes([0, 9, 1, 2]),
        "delta cut": bytes([1, 1, 0, 2, 3]),
        "single byte": bytes([5]),
        "empty": b"",
    }
    for nm, s in cases.items():
        data = bmp_file(40, 4, 3, 8, 1, s, pal_bytes(pal), 256)
        exp = rle_model(s, 4, 3, 8, lut)
        recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, data, expect_surface(exp, C.FMT_ARGB), "rle8 " + nm))
    # RLE with the wrong bit depth / pixel data offset past the end
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_file(40, 4, 3, 4, 1, b"\0\1", pal_bytes(pal[:16]), 16),
                       it.expect_err(C.ERR_UNSUPPORTED), "RLE8 with 4 bpp"))
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_file(40, 4, 3, 8, 2, b"\0\1", pal_bytes(pal), 256),
                       it.expect_err(C.ERR_UNSUPPORTED), "RLE4 with 8 bpp"))
    bad = bytearray(bmp_file(40, 4, 3, 8, 1, b"\0\1", pal_bytes(pal), 256))
    bad[10:14] = struct.pack("<I", len(bad) + 5)
    recs.append(it.Rec(OP_BMPDECFL, 0, bytes(bad), it.expect_err(C.ERR_TRUNC), "RLE offset past the end"))
    return recs, ffjobs


def suite_embed(runner_any, rng):
    """-> records; the expected pixels come from decoding the embedded files directly (op 240)."""
    recs = []
    r = np.random.default_rng(rng.randrange(1 << 30))
    st = src_image(rng, 29, 17, "alpha")
    from png_check import png_make as _unused  # noqa: F401  (keep the import graph simple)
    png = it.png_make(np.ascontiguousarray(st[..., [2, 1, 0, 3]]))
    res, jpg = runner_any.one(21, 80, struct.pack("<III", 29, 17, C.FMT_XRGB) + st.tobytes())
    files = [("png", 5, png)]
    if res == 0:
        files.append(("jpeg", 4, jpg))
    for nm, comp, emb in files:
        for flags in (0, C.IMGD_STRAIGHT):
            base = runner_any.one(it.OP_DECEX, 0, it.decex_input(emb, flags=flags))
            if base[0] != 0:
                continue
            exp = it.parse_decex(base[1])["px"]
            for si, size in (("exact", len(emb)), ("0", 0), ("big", len(emb) + 1000)):
                data = bmp_file(40, 29, 17, 0, comp, emb, size_image=size)
                recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=flags),
                                   it.expect_decex(exp, fmt=C.FMT_ARGB if flags else C.FMT_PARGB),
                                   f"BI_{nm.upper()} size {si} flags {flags}"))
            data = bmp_file(124, 29, -17, 0, comp, emb)
            recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT if flags else 0, data,
                               expect_surface(exp, C.FMT_ARGB if flags else C.FMT_PARGB), f"BI_{nm.upper()} V5 fl"))
        recs.append(it.Rec(OP_BMPDECFL, 0, bmp_file(40, 29, 17, 0, comp, emb[: len(emb) // 2]),
                           it.expect_any(it.expect_err(C.ERR_TRUNC), lambda r_, o_: None if r_ == 0 else "err"),
                           f"BI_{nm.upper()} truncated"))
    # the embedded file's size counts (ID_MAXPX), not the BMP header's
    big = it.png_make(np.zeros((64, 64, 4), np.uint8))
    tiny = bmp_file(40, 1, 1, 0, 5, big)
    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(tiny, maxpx=1000), it.expect_err(C.ERR_LIMIT),
                       "BI_PNG header 1x1, file 64x64, maxpx 1000"))
    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(tiny, flags=C.IMGD_PROBE),
                       it.expect_decex(None, meta=dict(w=64, h=64, channels=4)), "BI_PNG probe size"))
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_file(40, 3, 3, 0, 5, b"not a png at all"),
                       it.expect_err(C.ERR_HDR), "BI_PNG garbage"))
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_file(40, 3, 3, 0, 4, b"\xff\xd8\xff\xe0"),
                       it.expect_err(C.ERR_TRUNC, C.ERR_HDR), "BI_JPEG garbage"))
    _ = r
    return recs


def suite_unsup(rng):
    recs = []
    idx = np.zeros((4, 8), np.int64)
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_make(idx, 1, pal=[(0, 0, 0), (255, 255, 255)], hsz=64, comp=3),
                       it.expect_err(C.ERR_UNSUPPORTED), "OS/2 Huffman 1D"))
    bgr = np.zeros((4, 8, 3), np.int64)
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_make(bgr, 24, hsz=64, comp=4),
                       it.expect_err(C.ERR_UNSUPPORTED), "OS/2 RLE24"))
    for comp in (11, 12, 13, 7, 100):
        recs.append(it.Rec(OP_BMPDECFL, 0, bmp_file(40, 4, 4, 8, comp, bytes(64), pal_bytes([(1, 2, 3)] * 4), 4),
                           it.expect_err(C.ERR_UNSUPPORTED), f"compression {comp}"))
    # OS/2 64-byte header with plain data still decodes
    recs.append(it.Rec(OP_BMPDECFL, 0, bmp_make(bgr, 24, hsz=64),
                       expect_surface(np.concatenate([bgr, np.full((4, 8, 1), 255)], -1).astype(np.uint8),
                                      C.FMT_PARGB), "OS/2 64-byte header BI_RGB"))
    return recs


def v5_icc_file(icc, w=6, h=4, cstype=b"DEBM", ppm=(3780, 11811)):
    """V5 BMP (24 bit) with an embedded profile after the pixel data."""
    bgr = np.full((h, w, 3), 90, np.uint8)
    stride = ((w * 24 + 31) // 32) * 4
    data = b"".join(bgr[y].tobytes() + b"\0" * (stride - w * 3) for y in range(h))
    ext = bytearray(84)                    # header bytes 40..123
    ext[16:20] = cstype                     # bV5CSType at 56 (stored as a little-endian u32)
    prof_off = 124 + len(data)              # from the start of the info header
    struct.pack_into("<II", ext, 72, prof_off, len(icc))
    return bmp_file(124, w, h, 24, 0, data, extra_hdr=bytes(ext), tail=icc, ppm=ppm)


def suite_meta(rng):
    recs = []
    icc = fake_icc(700, 11)

    def chk(res, out, want_icc=icc, srgb=False):
        if res != 0:
            return f"error {it.err_name(res)}"
        m = meta_full(out)
        if m["icc"] != want_icc:
            return f"icc {len(m['icc'])} bytes"
        if (m["dpix"], m["dpiy"], m["resunit"]) != (ppm_dpi(3780), ppm_dpi(11811), C.IMR_CM):
            return f"dpi {m['dpix'] / 65536:.2f} {m['dpiy'] / 65536:.2f}"
        if bool(m["flags"] & C.IMF_SRGB) != srgb:
            return f"sRGB flag {m['flags']:#x}"
        return None
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC, v5_icc_file(icc), chk, "V5 embedded ICC"))
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC | C.IMGD_PROBE, v5_icc_file(icc), chk, "V5 embedded ICC probe"))
    recs.append(it.Rec(OP_METAFULL, 0, v5_icc_file(icc), lambda r, o: chk(r, o, b""), "V5 ICC not asked"))
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC, v5_icc_file(icc, cstype=b"BGRs"),
                       lambda r, o: chk(r, o, b"", True), "V5 sRGB"))
    cut = v5_icc_file(icc)[:-100]
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC, cut, lambda r, o: chk(r, o, b""), "V5 ICC past the end"))
    return recs


def bmp_read_any(data):
    """Our encoder's BMP -> dict(bpp, hsz, ncol, ppm, bgra straight)."""
    if data[:2] != b"BM":
        raise ValueError("magic")
    size, _, _, off = struct.unpack_from("<IHHI", data, 2)
    if size != len(data):
        raise ValueError("size field")
    hsz, w, h, planes, bpp, comp, isz, px, py, used = struct.unpack_from("<IiiHHIIiiI", data, 14)
    stride = ((w * bpp + 31) // 32) * 4
    ncol = used if bpp == 8 else 0
    if off != 14 + hsz + 4 * ncol or off + stride * h != len(data) or isz != stride * h:
        raise ValueError("layout")
    out = np.zeros((h, w, 4), np.uint8)
    pal = np.frombuffer(data, np.uint8, 4 * ncol, 14 + hsz).reshape(-1, 4) if ncol else None
    for y in range(h):
        row = data[off + (h - 1 - y) * stride: off + (h - y) * stride]
        if bpp == 24:
            out[y, :, :3] = np.frombuffer(row, np.uint8, w * 3).reshape(w, 3)
            out[y, :, 3] = 255
        elif bpp == 32:
            if hsz != 108 or comp != 3:
                raise ValueError("32-bit header")
            out[y] = np.frombuffer(row, np.uint8, w * 4).reshape(w, 4)
        elif bpp == 8:
            if hsz != 40 or comp != 0 or not 1 <= ncol <= 256:
                raise ValueError("8-bit header")
            idx = np.frombuffer(row, np.uint8, w)
            if idx.max() >= ncol:
                raise ValueError("index past the palette")
            out[y, :, :3] = pal[idx, :3]
            out[y, :, 3] = 255
        else:
            raise ValueError(f"bpp {bpp}")
    return dict(bpp=bpp, hsz=hsz, ncol=ncol, ppm=(px, py), bgra=out)


def expect_bmp(check):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        try:
            d = bmp_read_any(out)
        except Exception as e:  # noqa: BLE001
            return f"invalid BMP: {e}"
        return check(d)
    return chk


def suite_enc(rng):
    recs = []
    for (w, h) in ((1, 1), (3, 2), (17, 5), (64, 33)):
        for kind in ("few", "photo", "alpha", "binary"):
            st = src_image(rng, w, h, kind)
            for sfmt in (C.FMT_XRGB, C.FMT_PARGB, C.FMT_ARGB, C.FMT_A8):
                px, seen = as_input(st, sfmt)
                sin = it.surface_input(px, sfmt)
                nm = f"{w}x{h} {kind} fmt{sfmt}"
                opaque = bool(np.all(seen[..., 3] == 255))
                white = over_white(seen)

                def c32(d, seen=seen, sfmt=sfmt):
                    if d["bpp"] != 32:
                        return f"bpp {d['bpp']}"
                    g, e = d["bgra"].copy(), seen.copy()
                    if sfmt != C.FMT_ARGB:
                        g[g[..., 3] == 0] = 0
                        e[e[..., 3] == 0] = 0
                    return None if np.array_equal(g, e) else f"32-bit pixels (max {it.max_abs(g, e)})"

                def c24(d, white=white):
                    if d["bpp"] != 24:
                        return f"bpp {d['bpp']}"
                    return None if np.array_equal(d["bgra"], white) else f"24-bit pixels (max {it.max_abs(d['bgra'], white)})"

                def c8(d, white=white, level=0):
                    if d["bpp"] != 8:
                        return f"bpp {d['bpp']}"
                    ncol = len(np.unique(white.reshape(-1, 4).view(np.uint32)))
                    p = it.psnr(d["bgra"], white)
                    if ncol <= 256 and p < 99:
                        return f"not exact with {ncol} colours"
                    return None if p >= 22 else f"PSNR {p:.1f}"

                def cauto(d, seen=seen, white=white, opaque=opaque, c32=c32):
                    ncol = len(np.unique(white.reshape(-1, 4).view(np.uint32)))
                    if not opaque:
                        return c32(d)
                    want = 8 if ncol <= 256 else 24
                    if d["bpp"] != want:
                        return f"auto bpp {d['bpp']} != {want}"
                    return None if np.array_equal(d["bgra"], white) else "auto not lossless"
                recs.append(it.Rec(OP_BMPENCCFG, 0, struct.pack("<4I", 32, 0, 0, 0) + sin, expect_bmp(c32), "32 " + nm))
                recs.append(it.Rec(OP_BMPENCCFG, 0, struct.pack("<4I", 24, 0, 0, 0) + sin, expect_bmp(c24), "24 " + nm))
                for level in (0, 8):
                    recs.append(it.Rec(OP_BMPENCCFG, 0, struct.pack("<4I", 8, level | 77 << 8, 0, 0) + sin,
                                       expect_bmp(c8), f"8 L{level} " + nm))
                recs.append(it.Rec(OP_BMPENCCFG, 0, struct.pack("<4I", 0, 0, 3780, 1234) + sin,
                                   expect_bmp(lambda d, f=cauto: f(d) or (None if d["ppm"] == (3780, 1234) else "ppm")),
                                   "auto " + nm))
                recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(C.IMGF_BMP, bitdepth=8, dither=1) + sin, expect_bmp(c8),
                                   "img_encode 8 " + nm))
                recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(C.IMGF_BMP, bitdepth=24) + sin, expect_bmp(c24),
                                   "img_encode 24 " + nm))
                recs.append(it.Rec(it.OP_ENCODE, 0, imgopt(C.IMGF_BMP) + sin,
                                   expect_bmp(lambda d, f=cauto: f(d) or (None if d["ppm"] == (2835, 2835) else "ppm")),
                                   "img_encode auto " + nm))
    return recs


def classic_identity(runner, out, rng, suite):
    ref = it.ROOT / "build" / "ca" / "ref" / "codectest_c0.exe"
    if not ref.exists():
        suite.notes.append("classic identity: reference binary missing, skipped")
        return
    rr = it.Runner("win", out, work_name="bmp_ref")
    rr.exe = ref
    recs = []
    for (w, h) in ((1, 1), (5, 3), (40, 17)):
        for kind in ("few", "photo", "alpha", "binary"):
            st = src_image(rng, w, h, kind)
            for sfmt in (C.FMT_XRGB, C.FMT_PARGB, C.FMT_A8):
                px, _ = as_input(st, sfmt)
                recs.append(it.Rec(9, 0, it.surface_input(px, sfmt), None, f"classic {w}x{h} {kind} {sfmt}"))
                recs.append(it.Rec(7, 0, bmp_make(px.reshape(h, w, -1)[..., 0] if px.ndim == 2 else
                                                  px.view("<u4").reshape(h, w), 32 if px.ndim == 3 else 8,
                                                  pal=[(i, i, i) for i in range(256)] if px.ndim == 2 else None),
                                   None, f"classic decode {w}x{h} {kind} {sfmt}"))
    new = runner.batch(recs)
    old = rr.batch(recs)
    for r, a, b in zip(recs, new, old):
        suite.check(a is not None and b is not None and a == b, f"classic output differs: {r.name}")


def suite_fuzz(rng, count):
    bases = []
    pal = [(i, 255 - i, i // 2) for i in range(256)]
    bases.append(bmp_file(40, 13, 7, 8, 1, rle_random(rng, 13, 7, 8), pal_bytes(pal), 256))
    bases.append(bmp_file(40, 9, 5, 4, 2, rle_random(rng, 9, 5, 4), pal_bytes(pal[:16]), 16))
    bases.append(v5_icc_file(fake_icc(300, 5)))
    st = src_image(rng, 9, 5, "alpha")
    bases.append(bmp_file(40, 9, 5, 0, 5, it.png_make(np.ascontiguousarray(st[..., [2, 1, 0, 3]]))))
    px = np.random.default_rng(3).integers(0, 1 << 32, (5, 7), dtype=np.uint64)
    bases.append(bmp_make(px, 32, hsz=124, comp=3, masks=[0xFF0000, 0xFF00, 0xFF, 0xFF000000]))
    recs = []
    for i in range(count):
        b = bases[i % len(bases)]
        m = it.mutate(rng, b, keep=2)
        if i % 5 == 0:
            m = bytearray(m)
            for _ in range(2):
                off = rng.choice((10, 14, 18, 22, 28, 30, 34, 46, 70, 126, 130))
                if off + 4 <= len(m):
                    m[off:off + 4] = struct.pack("<I", rng.choice((0, 1, 2, 4, 5, 64, 124, 0xFFFFFFFF,
                                                                   0x7FFFFFFF, rng.getrandbits(32))))
            m = bytes(m)
        flags = [C.IMGD_STRAIGHT, 0, C.IMGD_ICC | C.IMGD_STRAIGHT][i % 3]
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(m, flags=flags), None, f"fuzz#{i}"))
        if i % 3 == 0:
            recs.append(it.Rec(OP_BMPDECFL, C.JDF_STRAIGHT, m, None, f"fuzz#{i} fl"))
    return recs


def fuzz_check(res, out):
    return None if res == 0 or C.ERR_LIMIT <= res <= -1 else f"bad result {res}"


def main():
    ap = it.argparser(__doc__, SUITES)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        it.build_codectest(targets, out)
    runners = {t: it.Runner(t, out, work_name="bmpcheck") for t in targets}
    ext_dir = out / "bmpcheck"
    ext_dir.mkdir(parents=True, exist_ok=True)
    use_ext = not a.no_ext and it.have_exe("ffmpeg")
    gen = []
    ffjobs = []
    if want("straight"):
        gen.append(("straight", suite_straight(random.Random(a.seed + 1))))
    if want("rle"):
        recs, ffjobs = suite_rle(random.Random(a.seed + 2), ext_dir, use_ext)
        gen.append(("rle", recs))
    if want("embed"):
        gen.append(("embed", suite_embed(runners[targets[0]], random.Random(a.seed + 3))))
    if want("unsup"):
        gen.append(("unsup", suite_unsup(random.Random(a.seed + 4))))
    if want("meta"):
        gen.append(("meta", suite_meta(random.Random(a.seed + 5))))
    if want("enc"):
        gen.append(("enc", suite_enc(random.Random(a.seed + 6))))
    if want("fuzz") and a.fuzz:
        recs = suite_fuzz(random.Random(a.seed + 7), a.fuzz)
        for r in recs:
            r.check = fuzz_check
        gen.append(("fuzz", recs))
    suites = []
    for t, runner in runners.items():
        for name, recs in gen:
            s = it.Suite(name, t)
            it.run_suite(runner, s, recs)
            suites.append(s)
        if want("enc") and t == "win":
            s = it.Suite("classic", t)
            classic_identity(runner, out, random.Random(a.seed + 8), s)
            suites.append(s)
    if ffjobs:
        s = it.Suite("ffmpeg", "host")
        for f, full, nm in ffjobs:
            got = it.ffmpeg_decode(f)
            if got is None:
                s.fail(f"ffmpeg cannot decode {nm}")
                continue
            s.check(np.array_equal(got[..., :3], full[..., :3]), f"{nm}: max |d| {it.max_abs(got[..., :3], full[..., :3])}")
        suites.append(s)
    if want("enc") and not a.no_ext and it.build.IS_WIN_HOST and "win" in runners:
        s = it.Suite("wic", "win")
        wic = it.Wic(out / "win" / "bmpcheck" / "wic")
        jobs = []
        r2 = random.Random(a.seed + 9)
        for depth in (0, 8, 24, 32):
            for kind in ("few", "photo", "alpha"):
                st = src_image(r2, 37, 21, kind)
                res, data = runners["win"].one(OP_BMPENCCFG, 0, struct.pack("<4I", depth, 8, 0, 0) +
                                               it.surface_input(st, C.FMT_ARGB))
                if res != 0:
                    s.fail(f"encode {depth} {kind}: {res}")
                    continue
                f = out / "win" / "bmpcheck" / f"wic_{depth}_{kind}.bmp"
                f.write_bytes(data)
                jobs.append((wic.decode(f), bmp_read_any(data), f"{depth} {kind}"))
        wic.run()
        for i, d, nm in jobs:
            r = wic.result(i)
            if r is None:
                s.fail(f"WIC cannot decode {nm}")
                continue
            got, exp = r[3], d["bgra"]
            if d["bpp"] == 32:
                got, exp = it.premul(got), it.premul(exp)        # WIC may drop colour at alpha 0
            s.check(np.array_equal(got, exp), f"WIC decode differs: {nm} (max {it.max_abs(got, exp)})")
        suites.append(s)
    return 0 if it.report(suites, "bmp_check") else 1


if __name__ == "__main__":
    sys.exit(main())
