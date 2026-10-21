#!/usr/bin/env python3
"""Checks of the TIFF codec (ASMLIB/src/img/tiff.s, tiff_ccitt.s) through apps/codectest.

Suites
  gen      Python-written TIFFs: every photometric (WhiteIsZero, BlackIsZero, RGB, palette,
           CMYK, YCbCr 1x1 and subsampled, CIELab), 1..32-bit integers (unsigned / signed),
           16 / 24 / 32-bit floats, alpha (ExtraSamples 0 / 1 / 2 / none), strips and tiles,
           planar 1 / 2, II / MM, BigTIFF, compression none / LZW (and old-style LZW) /
           Deflate (8, 32946) / PackBits, predictor 2 (8 / 16 / 32 / bit-packed) and 3,
           FillOrder 2 -> exact comparison with a numpy model of the decoder rules; the
           model is cross-checked against WIC (paint.net's decoder) and libtiff
  wic      WPF TiffBitmapEncoder files (Compression None / Ccitt3 / Ccitt4 / Lzw / Rle / Zip x
           BlackWhite / Gray8 / Gray16 / Bgra32 / Pbgra32 / Rgba64 / Prgba64 / Cmyk32) against
           WIC's own decode (CMYK against libtiff: WIC converts CMYK colour-managed)
  ffmpeg   ffmpeg's TIFF encoder (rgb24 rgba rgb48le rgba64le gray gray16le ya8 ya16le pal8
           monob monow yuv420p yuv422p yuv444p yuv410p yuv411p yuv440p x raw / packbits /
           lzw / deflate, -dpi 300) against WIC / libtiff
  libtiff  files written by libtiff (WSL, ctypes): CCITT RLE / T.4 1D / T.4 2D / T.4 2D with
           fill bits / T.6, FillOrder 1 / 2, WhiteIsZero / BlackIsZero; JPEG (YCbCr 2x2 /
           2x1 / 1x1, RGB, grey, tiles, restart markers via JPEGTables); predictor 2 at 16 bit,
           predictor 3; tiles, planar data -> against libtiff's TIFFReadRGBAImage
  encode   img_encode TIFF: bit depth auto / 32 / 24 / 8, compression default / none / LZW /
           Deflate / PackBits, dithering -> our decoder (exact), WIC, ffmpeg and libtiff;
           ICC / XMP / dpi through IO_META (op 61); file structure (tags, strips ~64 KB)
  meta     IMGMETA (size, depth, model, channels, flags, orientation, dpi, pages), blobs (ICC,
           XMP, EXIF block -> exif_read), IMGD_PROBE, img_probe, pages / frame selection,
           orientations 1..8 applied by the dispatcher, PARGB output
  errors   unsupported compressions / photometrics / depths, malformed and truncated files,
           huge counts, IFD cycles, ID_MAXPX, cancel
  par      >= 1 MP files with many strips / tiles (parallel decoder): every layout exact,
           identical with --threads 1, the lowest failing strip's error, cancel
  fuzz     mutated files (truncation, bit flips, runs) + structural mutants on both targets:
           any result, no crash, no hang (inputs end at a guard page)
  perf     decode time of a 12 MP LZW and Deflate RGB TIFF (target < 250 ms), encode times

  python tools/tiff_check.py --target both --out build/ct
  python tools/tiff_check.py --target win --only gen,encode --no-build

Rounding rules: RGB-type 16-bit samples v >> 8 and
palette entries trunc(v / 257) (both exactly WIC), 16-bit grey round(v / 257) (WIC: +-1),
other integer depths round(v * 255 / (2^n - 1)), floats linear -> sRGB with the library's
4096-entry table (WIC: +-1), alpha linear; CMYK like libtiff; associated alpha: 8 bit -> the
library's unpremultiply (WIC truncates: +-1), deeper -> unpremultiplied at full precision.
"""

import json
import random
import struct
import subprocess
import sys
import zlib
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402

C = it.C
Rec = it.Rec
SUITES = "gen,wic,ffmpeg,libtiff,encode,meta,errors,par,fuzz,perf"
OP_TDEC, OP_ENCMETA, OP_TENC = 60, 61, 62
STRAIGHT = C.IMGD_STRAIGHT

# ================================================================== TIFF writer

TSZ = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 6: 1, 7: 1, 8: 2, 9: 4, 10: 8, 11: 4, 12: 8, 13: 4,
       16: 8, 17: 8, 18: 8}
TFMT = {1: "B", 2: "B", 3: "H", 4: "I", 6: "b", 7: "B", 8: "h", 9: "i", 11: "f", 12: "d", 13: "I",
        16: "Q", 17: "q", 18: "Q"}


def enc_vals(bo, typ, v):
    if isinstance(v, (bytes, bytearray)):
        return bytes(v)
    out = b""
    for x in v:
        if typ in (5, 10):
            out += struct.pack(bo + ("II" if typ == 5 else "ii"), *x)
        else:
            out += struct.pack(bo + TFMT[typ], x)
    return out


def write_tiff(pages, bo="<", big=False, nexts=None, want_pos=False):
    """pages: [{'tags': {tag: (type, values)}, 'chunks': [bytes], 'tiled': bool,
    'offtype': 3 / 4 / 16, 'override': {tag: (type, values) or None}}].  Chunk offsets and
    byte counts (273 / 279 or 324 / 325) come from 'chunks'.  nexts: next-IFD index per page
    (None = end, negative -k = raw offset k)."""
    esz, inl, cnt_fmt, ofmt = (20, 8, "Q", "Q") if big else (12, 4, "H", "I")
    out = bytearray(b"II" if bo == "<" else b"MM")
    out += struct.pack(bo + "HHHQ", 43, 8, 0, 0) if big else struct.pack(bo + "HI", 42, 0)
    ifd_pos, fix = [], []
    for pi, pg in enumerate(pages):
        offs, cnts = [], []
        for c in pg.get("chunks", []):
            if len(out) & 1:
                out += b"\0"
            offs.append(len(out))
            cnts.append(len(c))
            out += c
        tags = dict(pg["tags"])
        if "chunks" in pg:
            ot = pg.get("offtype", 16 if big else 4)
            if pg.get("tiled"):
                tags[324], tags[325] = (ot, offs), (ot, cnts)
            else:
                tags[273], tags[279] = (ot, offs), (ot, cnts)
        for k, v in pg.get("override", {}).items():
            if v is None:
                tags.pop(k, None)
            else:
                tags[k] = v
        if len(out) & 1:
            out += b"\0"
        pos = len(out)
        ifd_pos.append(pos)
        items = sorted(tags.items())
        data_pos = pos + (8 if big else 2) + len(items) * esz + (8 if big else 4)
        ent, data = bytearray(), bytearray()
        for tag, (typ, v) in items:
            b = enc_vals(bo, typ, v)
            ent += struct.pack(bo + "HH" + ofmt, tag, typ, len(b) // TSZ[typ])
            if len(b) <= inl:
                ent += b + b"\0" * (inl - len(b))
            else:
                ent += struct.pack(bo + ofmt, data_pos + len(data))
                data += b + (b"\0" if len(b) & 1 else b"")
        out += struct.pack(bo + cnt_fmt, len(items)) + ent
        fix.append((len(out), pi))
        out += struct.pack(bo + ofmt, 0) + data
    struct.pack_into(bo + ("Q" if big else "I"), out, 8 if big else 4, ifd_pos[0] if ifd_pos else 0)
    for p, pi in fix:
        nx = nexts[pi] if nexts is not None else (pi + 1 if pi + 1 < len(pages) else None)
        val = 0 if nx is None else (ifd_pos[nx] if nx >= 0 else -nx)
        struct.pack_into(bo + ofmt, out, p, val)
    return (bytes(out), ifd_pos) if want_pos else bytes(out)


def packbits(data):
    out, i, n = bytearray(), 0, len(data)
    while i < n:
        j = i + 1
        while j < n and j - i < 128 and data[j] == data[i]:
            j += 1
        if j - i >= 2:
            out += bytes([(257 - (j - i)) & 0xFF, data[i]])
            i = j
            continue
        j = i + 1
        while j < n and j - i < 128 and not (j + 1 < n and data[j] == data[j + 1]):
            j += 1
        out.append(j - i - 1)
        out += data[i:j]
        i = j
    return bytes(out)


def lzw(data, compat=False):
    """TIFF LZW like libtiff (MSB first, one-code-early width change, clear at 4094);
    compat=True: the old bit-reversed variant (LSB first, no early change)."""
    out = bytearray()
    acc, nacc = 0, 0

    def put(code, nb):
        nonlocal acc, nacc
        if compat:
            acc |= code << nacc
            nacc += nb
            while nacc >= 8:
                out.append(acc & 0xFF)
                acc >>= 8
                nacc -= 8
        else:
            acc = (acc << nb) | code
            nacc += nb
            while nacc >= 8:
                out.append((acc >> (nacc - 8)) & 0xFF)
                nacc -= 8
                acc &= (1 << nacc) - 1
    extra = 1 if compat else 0
    st = {"nb": 9, "nxt": 258, "table": {}}
    put(256, 9)

    def bump():
        st["nxt"] += 1
        if st["nxt"] == 4094:
            put(256, st["nb"])
            st.update(nb=9, nxt=258, table={})
        elif st["nxt"] >= (1 << st["nb"]) + extra and st["nb"] < 12:
            st["nb"] += 1
    w = b""
    for ch in data:
        c = bytes([ch])
        wc = w + c
        if len(wc) == 1 or wc in st["table"]:
            w = wc
            continue
        put(st["table"][w] if len(w) > 1 else w[0], st["nb"])
        st["table"][wc] = st["nxt"]
        bump()
        w = c
    if w:
        put(st["table"][w] if len(w) > 1 else w[0], st["nb"])
        bump()
    put(257, st["nb"])
    if nacc:
        out.append(acc & 0xFF if compat else (acc << (8 - nacc)) & 0xFF)
    return bytes(out)


REV8 = bytes(int(f"{i:08b}"[::-1], 2) for i in range(256))


def compress(raw, comp):
    if comp == 1:
        return raw
    if comp == 5:
        return lzw(raw)
    if comp == -5:
        return lzw(raw, compat=True)
    if comp in (8, 32946):
        return zlib.compress(raw, 6)
    if comp == 32773:
        return packbits(raw)
    raise ValueError(comp)


def f24_bits(f):
    """float32 array -> float24 bit patterns (1 sign, 7 exponent bias 63, 16 mantissa;
    values must be representable: they come from f24_round)."""
    b = np.asarray(f, np.float32).view(np.uint32).astype(np.int64)
    s = (b >> 31) & 1
    e = (b >> 23) & 0xFF
    m = b & 0x7FFFFF
    e24 = np.where(e == 0, 0, e - 127 + 63)
    return ((s << 23) | (np.clip(e24, 0, 127) << 16) | (m >> 7)).astype(np.uint32)


def f24_round(f):
    """float32 values representable in float24 (mantissa truncated, exponent in range)."""
    f = np.asarray(f, np.float32)
    b = f.view(np.uint32) & np.uint32(0xFFFFFF80)
    out = b.view(np.float32).copy()
    a = np.abs(out)
    out[(a < 2.0 ** -61) & (a != 0)] = 0
    return out


def serialize(vals, bps, sfmt, bo):
    """2-D array (rows, samples) -> bytes: 8 / 16 / 32 / 24(float) in byte order bo, other
    depths MSB-first bit-packed per row."""
    rows = []
    for row in vals:
        if bps == 8:
            rows.append(np.asarray(row, np.uint8).tobytes())
        elif bps in (16, 32) and sfmt == 3:
            rows.append(np.asarray(row, {16: np.float16, 32: np.float32}[bps]).astype(
                {16: bo + "f2", 32: bo + "f4"}[bps]).tobytes())
        elif bps == 16:
            rows.append(np.asarray(row, np.uint64).astype(np.uint16).astype(bo + "u2").tobytes())
        elif bps == 32:
            rows.append(np.asarray(row, np.uint64).astype(np.uint32).astype(bo + "u4").tobytes())
        elif bps == 24:
            v = np.asarray(row, np.uint64)
            b = np.stack([(v >> 16) & 255, (v >> 8) & 255, v & 255], -1).astype(np.uint8)
            if bo == "<":
                b = b[:, ::-1]
            rows.append(b.tobytes())
        else:
            acc, n, out = 0, 0, bytearray()
            for x in row:
                acc = (acc << bps) | int(x)
                n += bps
                while n >= 8:
                    out.append((acc >> (n - 8)) & 0xFF)
                    n -= 8
                    acc &= (1 << n) - 1
            if n:
                out.append((acc << (8 - n)) & 0xFF)
            rows.append(bytes(out))
    return b"".join(rows)


def pred2_rows(vals, spp, bps):
    a = np.asarray(vals, np.int64)
    d = a.copy()
    d[:, spp:] = a[:, spp:] - a[:, :-spp]
    return d % (1 << bps)


def pred3_rows(fvals, spp, bps):
    """float rows (rows, n) -> predictor-3 byte rows (byte planes MSB first, differenced)."""
    dt = {16: ">f2", 24: None, 32: ">f4"}[bps]
    out = bytearray()
    for row in fvals:
        n = len(row)
        if bps == 24:
            v = f24_bits(row).astype(np.int64)
            b = np.stack([(v >> 16) & 255, (v >> 8) & 255, v & 255], -1).astype(np.uint8)
        else:
            b = np.frombuffer(np.asarray(row, {16: np.float16, 32: np.float32}[bps]).astype(dt).tobytes(),
                              np.uint8).reshape(n, bps // 8)
        planes = b.T.reshape(-1).astype(np.int64)
        d = planes.copy()
        d[spp:] = planes[spp:] - planes[:-spp]
        out += (d % 256).astype(np.uint8).tobytes()
    return bytes(out)


def make_tiff(samples, bps, phot, sfmt=1, comp=1, pred=1, planar=1, tile=None, rps=None, bo="<",
              big=False, fill=1, extra=None, tags=None, cmap=None, pages=None, offtype=None):
    """samples (h, w, spp): ints (sfmt 1 / 2: values as unsigned bit patterns) or floats (sfmt 3).
    Returns the TIFF bytes; `pages` (list of extra make_page dicts) adds further pages."""
    pg = make_page(samples, bps, phot, sfmt, comp, pred, planar, tile, rps, bo, fill, extra, tags,
                   cmap, offtype)
    return write_tiff([pg] + list(pages or []), bo=bo, big=big)


def make_page(samples, bps, phot, sfmt=1, comp=1, pred=1, planar=1, tile=None, rps=None, bo="<",
              fill=1, extra=None, tags=None, cmap=None, offtype=None):
    s = np.asarray(samples)
    if s.ndim == 2:
        s = s[..., None]
    h, w, spp = s.shape
    if tile:
        tw, th = tile
        ntx, nty = -(-w // tw), -(-h // th)
        regions = [(tx * tw, ty * th, tw, th) for ty in range(nty) for tx in range(ntx)]
    else:
        r = rps or h
        regions = [(0, y, w, min(r, h - y)) for y in range(0, h, r)]
    planes = [list(range(spp))] if planar == 1 else [[k] for k in range(spp)]
    chunks = []
    for pl in planes:
        for (x0, y0, cw, ch) in regions:
            blk = np.zeros((ch, cw, len(pl)), s.dtype)
            sub = s[y0:y0 + ch, x0:x0 + cw][..., pl]
            blk[:sub.shape[0], :sub.shape[1]] = sub
            rows = blk.reshape(ch, cw * len(pl))
            if pred == 3:
                raw = pred3_rows(rows.astype(np.float32), len(pl), bps)
            else:
                if sfmt == 3:
                    rows = np.asarray(rows, {16: np.float16, 32: np.float32, 24: np.float32}[bps])
                    if bps == 24:
                        rows = f24_bits(rows)
                    else:
                        rows = rows.view({16: np.uint16, 32: np.uint32}[bps])
                if pred == 2:
                    rows = pred2_rows(rows, len(pl), bps)
                raw = serialize(rows, bps, 1 if pred == 2 or sfmt == 3 else sfmt, bo)
            c = compress(raw, comp)
            if fill == 2:
                c = c.translate(REV8)
            chunks.append(c)
    t = {256: (4, [w]), 257: (4, [h]), 258: (3, [bps] * spp), 259: (3, [abs(comp)]),
         262: (3, [phot]), 277: (3, [spp]), 284: (3, [planar])}
    if tile:
        t[322], t[323] = (3, [tile[0]]), (3, [tile[1]])
    else:
        t[278] = (4, [rps or h])
    if pred != 1:
        t[317] = (3, [pred])
    if sfmt != 1:
        t[339] = (3, [sfmt] * spp)
    if fill != 1:
        t[266] = (3, [fill])
    if extra is not None:
        t[338] = (3, list(extra))
    if cmap is not None:
        t[320] = (3, [int(x) for x in cmap])
    for k, v in (tags or {}).items():
        if v is None:
            t.pop(k, None)
        else:
            t[k] = v
    pg = {"tags": t, "chunks": chunks, "tiled": bool(tile)}
    if offtype:
        pg["offtype"] = offtype
    return pg


def ycc_sub_tiff(Y, Cb, Cr, sub, comp=1, rps=None, tile=None, bo="<", tags=None):
    """YCbCr with subsampling sub = (h, v): Y (H, W), Cb / Cr (block rows, block cols) of the
    whole image; blocks past the image edge are padded with 0."""
    hh, vv = sub
    H, W = Y.shape
    if tile:
        tw, th = tile
        regions = [(tx * tw, ty * th, tw, th) for ty in range(-(-H // th)) for tx in range(-(-W // tw))]
    else:
        r = rps or H
        regions = [(0, y, W, min(r, H - y)) for y in range(0, H, r)]
    chunks = []
    for (x0, y0, cw, ch) in regions:
        out = bytearray()
        for by in range(-(-ch // vv)):
            for bx in range(-(-cw // hh)):
                for dy in range(vv):
                    for dx in range(hh):
                        yy, xx = y0 + by * vv + dy, x0 + bx * hh + dx
                        out.append(int(Y[yy, xx]) if yy < H and xx < W else 0)
                gy, gx = (y0 // vv) + by, (x0 // hh) + bx
                ok = gy < Cb.shape[0] and gx < Cb.shape[1]
                out += bytes([int(Cb[gy, gx]) if ok else 0, int(Cr[gy, gx]) if ok else 0])
        chunks.append(compress(bytes(out), comp))
    t = {256: (4, [W]), 257: (4, [H]), 258: (3, [8, 8, 8]), 259: (3, [abs(comp)]), 262: (3, [6]),
         277: (3, [3]), 284: (3, [1]), 530: (3, [hh, vv])}
    if tile:
        t[322], t[323] = (3, [tile[0]]), (3, [tile[1]])
    else:
        t[278] = (4, [rps or H])
    for k, v in (tags or {}).items():
        if v is None:
            t.pop(k, None)
        else:
            t[k] = v
    return write_tiff([{"tags": t, "chunks": chunks, "tiled": bool(tile)}], bo=bo)


# ================================================================== reference model

def lin_srgb_lut():
    out = []
    for i in range(4096):
        v = i / 4095
        s = v * 12.92 if v <= 0.0031308 else 1.055 * v ** (1 / 2.4) - 0.055
        out.append(min(255, int(255 * s + 0.5)))
    return np.array(out, np.uint8)


SRGB_LUT = lin_srgb_lut()


def f32_clamp(f):
    f = np.asarray(f, np.float32)
    f = np.where(np.isnan(f), np.float32(0), f)
    return np.minimum(np.maximum(f, np.float32(0)), np.float32(1)).astype(np.float32)


def f2u8(f):
    return (f32_clamp(f) * np.float32(255) + np.float32(0.5)).astype(np.float32).astype(np.int64)


def srgb8(f):
    idx = (f32_clamp(f) * np.float32(4095) + np.float32(0.5)).astype(np.float32).astype(np.int64) & 4095
    return SRGB_LUT[idx].astype(np.int64)


def norm(v, bps, gray16=False):
    v = np.asarray(v, np.int64)
    if bps == 8:
        return v
    if bps == 16:
        return (v * 255 + 32895) >> 16 if gray16 else v >> 8
    m = (1 << bps) - 1
    return (v * 510 + m) // (2 * m)


def ycc_tables(luma=(0.299, 0.587, 0.114), rbw=(0, 255, 128, 255, 128, 255)):
    """libtiff TIFFYCbCrToRGBInit in float32."""
    f32 = np.float32
    LR, LG, LB = (f32(x) for x in luma)
    rbw = [f32(x) for x in rbw]

    def fix(x):
        x = min(max(f32(x), f32(0)), f32(2))
        return int(float(x) * 65536.0 + 0.5)
    f1 = f32(2) - f32(2) * LR
    D1 = fix(f1)
    D2 = -fix(LR * f1 / LG)
    f3 = f32(2) - f32(2) * LB
    D3 = fix(f3)
    D4 = -fix(LB * f3 / LG)

    def code2v(c, RB, RW, CR):
        den = RW - RB
        if den == 0:
            den = f32(1)
        r = f32(f32(c - int(RB)) * f32(CR)) / den
        r = min(max(f32(r), f32(-4096)), f32(4096))
        return int(r)
    crr, cbb, crg, cbg, yt = [], [], [], [], []
    for i in range(256):
        x = i - 128
        cr = code2v(x, rbw[4] - f32(128), rbw[5] - f32(128), 127)
        cb = code2v(x, rbw[2] - f32(128), rbw[3] - f32(128), 127)
        crr.append((D1 * cr + 32768) >> 16)
        cbb.append((D3 * cb + 32768) >> 16)
        crg.append(D2 * cr)
        cbg.append(D4 * cb + 32768)
        yt.append(code2v(i, rbw[0], rbw[1], 255))
    return [np.array(t, np.int64) for t in (crr, cbb, crg, cbg, yt)]


def ycc_rgb(Y, Cb, Cr, tabs=None):
    crr, cbb, crg, cbg, yt = tabs or ycc_tables()
    Y, Cb, Cr = (np.asarray(a, np.int64) for a in (Y, Cb, Cr))
    y = yt[Y]
    r = np.clip(y + crr[Cr], 0, 255)
    g = np.clip(y + ((cbg[Cb] + crg[Cr]) >> 16), 0, 255)
    b = np.clip(y + cbb[Cb], 0, 255)
    return r, g, b


LABM = np.array([[3.021664, -1.61686649, -0.404708137], [-0.943728349, 1.91614137, 0.0275962655],
                 [0.0693696473, -0.228991373, 1.15918468]], np.float32)


def lab_rgb(q, bps):
    f32 = np.float32
    q = np.asarray(q, np.int64)
    L = q[..., 0].astype(np.float32)
    if bps == 8:
        L = L * f32(0.392156869)
        a = ((q[..., 1] + 128) % 256 - 128).astype(np.float32)
        b = ((q[..., 2] + 128) % 256 - 128).astype(np.float32)
    else:
        L = L * f32(0.00152590219)
        a = ((q[..., 1] + 32768) % 65536 - 32768).astype(np.float32) * f32(0.00390625)
        b = ((q[..., 2] + 32768) % 65536 - 32768).astype(np.float32) * f32(0.00390625)
    fy = ((L + f32(16)) * f32(0.00862068966)).astype(np.float32)
    fx = (a * f32(0.002) + fy).astype(np.float32)
    fz = (fy - b * f32(0.005)).astype(np.float32)

    def finv(t):
        cube = (t * (t * t).astype(np.float32)).astype(np.float32)
        lin = ((t - f32(0.137931034)) * f32(0.128418549)).astype(np.float32)
        return np.where(t > f32(0.206896552), cube, lin).astype(np.float32)
    X, Yv, Z = finv(fx), finv(fy), finv(fz)
    out = []
    for r in range(3):
        v = (LABM[r, 0] * X).astype(np.float32)
        v = (v + (LABM[r, 1] * Yv).astype(np.float32)).astype(np.float32)
        v = (v + (LABM[r, 2] * Z).astype(np.float32)).astype(np.float32)
        out.append(srgb8(v))
    return out


def half_f32(h):
    return np.asarray(h, np.uint16).view(np.float16).astype(np.float32)


def expected(samples, bps, phot, sfmt=1, alpha=None, assoc=False, cmap=None, luma=None, rbw=None):
    """Straight BGRA (h, w, 4) the decoder produces (IMGD_STRAIGHT).  samples: (h, w, spp) ints
    (bit patterns for sfmt 2) or floats (sfmt 3); alpha = alpha sample index or None."""
    s = np.asarray(samples)
    if s.ndim == 2:
        s = s[..., None]
    h, w, spp = s.shape
    flt = sfmt == 3
    if flt:
        f = np.asarray(s, np.float32)
        if bps == 16:
            f = np.asarray(s, np.float16).astype(np.float32)
    else:
        v = np.asarray(s, np.int64)
        if sfmt == 2 and phot != 8:
            v = v ^ (1 << (bps - 1))
    out = np.zeros((h, w, 4), np.int64)
    maxv = (1 << bps) - 1
    ncol = {0: 1, 1: 1, 2: 3, 3: 1, 5: 4, 6: 3, 8: 3}[phot]
    if alpha is not None and assoc and (bps != 8 or flt):
        if flt:
            a = f[..., alpha:alpha + 1]
            with np.errstate(divide="ignore", invalid="ignore"):
                f = f.copy()
                f[..., :ncol] = np.where(a > 0, (f[..., :ncol] / a).astype(np.float32), np.float32(0))
        else:
            a = v[..., alpha:alpha + 1].astype(object)          # Python ints: 32-bit products
            c = v[..., :ncol].astype(object)
            u = np.where(c >= a, maxv, (c * maxv + a // 2) // np.maximum(a, 1))
            u = np.where(a == 0, 0, u)
            v = v.copy()
            v[..., :ncol] = u.astype(np.int64)
    if alpha is not None:
        out[..., 3] = f2u8(f[..., alpha]) if flt else norm(v[..., alpha], bps)
    else:
        out[..., 3] = 255

    def nc(k):
        if flt:
            return srgb8(f[..., k])
        return norm(v[..., k], bps, gray16=phot in (0, 1))
    if phot in (0, 1):
        g = nc(0)
        if phot == 0:
            g = 255 - g
        out[..., 0] = out[..., 1] = out[..., 2] = g
    elif phot == 2:
        out[..., 2], out[..., 1], out[..., 0] = nc(0), nc(1), nc(2)
    elif phot == 3:
        n = len(cmap) // 3
        cm = np.asarray(cmap, np.int64)
        idx = v[..., 0]
        ok = idx < n
        ii = np.minimum(idx, n - 1)
        out[..., 2] = np.where(ok, cm[ii] // 257, 0)
        out[..., 1] = np.where(ok, cm[n + ii] // 257, 0)
        out[..., 0] = np.where(ok, cm[2 * n + ii] // 257, 0)
    elif phot == 5:
        c, m, y, k = (nc(i) for i in range(4))
        kk = 255 - k
        out[..., 2] = kk * (255 - c) // 255
        out[..., 1] = kk * (255 - m) // 255
        out[..., 0] = kk * (255 - y) // 255
    elif phot == 6:
        tabs = ycc_tables(luma or (0.299, 0.587, 0.114), rbw or (0, 255, 128, 255, 128, 255))
        r, g, b = ycc_rgb(nc(0), nc(1), nc(2), tabs)
        out[..., 2], out[..., 1], out[..., 0] = r, g, b
    elif phot == 8:
        r, g, b = lab_rgb(v[..., :3], bps)
        out[..., 2], out[..., 1], out[..., 0] = r, g, b
    res = out.astype(np.uint8)
    if alpha is not None and assoc and bps == 8 and not flt:
        pm = res.copy()                                  # stored premultiplied -> library unpremul
        pm[..., :3] = np.minimum(pm[..., :3], pm[..., 3:4])
        return it.unpremul(pm)
    return res


def ysub_expected(Y, Cb, Cr, sub, rbw=None):
    hh, vv = sub
    H, W = Y.shape
    cb = np.repeat(np.repeat(Cb, vv, 0), hh, 1)[:H, :W]
    cr = np.repeat(np.repeat(Cr, vv, 0), hh, 1)[:H, :W]
    r, g, b = ycc_rgb(Y, cb, cr, ycc_tables(rbw=rbw) if rbw else None)
    return np.stack([b, g, r, np.full_like(b, 255)], -1).astype(np.uint8)


def ysub_model_from_file(data):
    """Expected pixels of an uncompressed, strip-organised subsampled YCbCr TIFF from its blocks."""
    tags, _, _ = parse_ifd(data)
    W, H = tags[256][1][0], tags[257][1][0]
    hh, vv = tags.get(530, (3, [2, 2]))[1][:2]
    rbw = None
    if 532 in tags:
        rbw = [n / d for (n, d) in tags[532][1]]
    raw = b"".join(data[o:o + c] for o, c in zip(tags[273][1], tags[279][1]))
    nbx, nby = -(-W // hh), -(-H // vv)
    Y = np.zeros((nby * vv, nbx * hh), np.int64)
    Cb = np.zeros((nby, nbx), np.int64)
    Cr = np.zeros((nby, nbx), np.int64)
    bs = hh * vv + 2
    for by in range(nby):
        for bx in range(nbx):
            b = raw[(by * nbx + bx) * bs:(by * nbx + bx + 1) * bs]
            Y[by * vv:(by + 1) * vv, bx * hh:(bx + 1) * hh] = np.frombuffer(b[:hh * vv], np.uint8).reshape(vv, hh)
            Cb[by, bx], Cr[by, bx] = b[hh * vv], b[hh * vv + 1]
    return ysub_expected(Y[:H, :W], Cb, Cr, (hh, vv), rbw)


# ================================================================== external tools

WIC_PS = r"""
param([string]$jobs)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase
foreach ($line in [IO.File]::ReadAllLines($jobs)) {
  $a = $line.Split("`t")
  try {
    $w = [int]$a[2]; $h = [int]$a[3]; $st = $w * 4
    $raw = [IO.File]::ReadAllBytes($a[1])
    $src = [System.Windows.Media.Imaging.BitmapSource]::Create($w, $h, 96, 96, [System.Windows.Media.PixelFormats]::Bgra32, $null, $raw, $st)
    $pf = [System.Windows.Media.PixelFormats].GetProperty($a[5]).GetValue($null, $null)
    $conv = New-Object System.Windows.Media.Imaging.FormatConvertedBitmap($src, $pf, $null, 0)
    $enc = New-Object System.Windows.Media.Imaging.TiffBitmapEncoder
    $enc.Compression = [Enum]::Parse([System.Windows.Media.Imaging.TiffCompressOption], $a[6])
    $enc.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($conv))
    $fs = [IO.File]::Create($a[4]); $enc.Save($fs); $fs.Close()
  } catch {
    [IO.File]::WriteAllText(($a[4] + '.err'), $_.Exception.Message)
  }
}
"""


def wic_encode_batch(work, jobs):
    """jobs: [(bgra, out_path, pixel_format, compression)] -> list of bool (written)."""
    work = Path(work)
    work.mkdir(parents=True, exist_ok=True)
    lines = []
    for i, (bgra, out, pf, comp) in enumerate(jobs):
        raw = work / f"wicenc_{i}.raw"
        raw.write_bytes(np.ascontiguousarray(bgra, np.uint8).tobytes())
        h, w = bgra.shape[:2]
        Path(out).unlink(missing_ok=True)
        lines.append(f"x\t{raw.resolve()}\t{w}\t{h}\t{Path(out).resolve()}\t{pf}\t{comp}")
    ps = work / "wic_tiffenc.ps1"
    ps.write_text(WIC_PS, encoding="utf-8")
    jf = work / "wic_tiffenc.jobs"
    jf.write_text("\n".join(lines) + "\n", encoding="utf-8")
    subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(ps), str(jf)],
                   capture_output=True, timeout=1800)
    return [Path(o).exists() for (_, o, _, _) in jobs]


def wic_decode_batch(work, paths):
    """[(path, frame)] -> [(w, h, frames, px) or None]."""
    wic = it.Wic(Path(work) / "wicdec")
    ids = [wic.decode(p, fr) for p, fr in paths]
    wic.run()
    return [wic.result(i) for i in ids]


LT_PY = r'''
import ctypes, json, struct, sys
lib = ctypes.CDLL("libtiff.so.6")
lib.TIFFOpen.restype = ctypes.c_void_p
lib.TIFFOpen.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
lib.TIFFClose.argtypes = [ctypes.c_void_p]
lib.TIFFSetDirectory.argtypes = [ctypes.c_void_p, ctypes.c_uint16]
lib.TIFFReadRGBAImageOriented.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32,
                                          ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
lib.TIFFGetField.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p]
lib.TIFFWriteEncodedStrip.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_ssize_t]
lib.TIFFWriteEncodedStrip.restype = ctypes.c_ssize_t
lib.TIFFWriteEncodedTile.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_ssize_t]
lib.TIFFWriteEncodedTile.restype = ctypes.c_ssize_t
lib.TIFFWriteDirectory.argtypes = [ctypes.c_void_p]
lib.TIFFSetField.restype = ctypes.c_int
ERRH = ctypes.CFUNCTYPE(None, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p)
lib.TIFFSetErrorHandler.restype = ctypes.c_void_p
lib.TIFFSetWarningHandler.restype = ctypes.c_void_p
_nop = ERRH(lambda a, b, c: None)
lib.TIFFSetErrorHandler(_nop)
lib.TIFFSetWarningHandler(_nop)
keep = []
def setf(t, tag, val):
    if tag in (282, 283):
        return lib.TIFFSetField(t, ctypes.c_uint32(tag), ctypes.c_double(val))
    if tag == 338:
        arr = (ctypes.c_uint16 * len(val))(*val); keep.append(arr)
        return lib.TIFFSetField(t, ctypes.c_uint32(tag), ctypes.c_int(len(val)), arr)
    if tag == 530:
        return lib.TIFFSetField(t, ctypes.c_uint32(tag), ctypes.c_int(val[0]), ctypes.c_int(val[1]))
    if tag == 320:
        n = len(val) // 3
        arrs = [(ctypes.c_uint16 * n)(*val[i * n:(i + 1) * n]) for i in range(3)]; keep.append(arrs)
        return lib.TIFFSetField(t, ctypes.c_uint32(tag), *arrs)
    return lib.TIFFSetField(t, ctypes.c_uint32(tag), ctypes.c_int(val))
log = []
for job in json.loads(sys.stdin.read()):
    if job["op"] == "write":
        t = lib.TIFFOpen(job["path"].encode(), b"w")
        ok = 1 if t else 0
        if t:
            for pg in job["pages"]:
                for tag, val in pg["tags"]:
                    if not setf(t, tag, val):
                        ok = 0
                raw = bytes.fromhex(pg["data"])
                csz = pg["chunk"]
                fn = lib.TIFFWriteEncodedTile if pg.get("tiled") else lib.TIFFWriteEncodedStrip
                for i in range(-(-len(raw) // csz)):
                    chunk = raw[i * csz:(i + 1) * csz]
                    buf = ctypes.create_string_buffer(chunk, len(chunk))
                    if fn(t, i, buf, len(chunk)) < 0:
                        ok = 0
                if len(job["pages"]) > 1:
                    lib.TIFFWriteDirectory(t)
            lib.TIFFClose(t)
        log.append(ok)
    else:
        res = struct.pack("<III", 0, 0, 0)
        t = lib.TIFFOpen(job["path"].encode(), b"r")
        if t:
            if lib.TIFFSetDirectory(t, job.get("page", 0)):
                w = ctypes.c_uint32(); h = ctypes.c_uint32()
                lib.TIFFGetField(t, 256, ctypes.byref(w)); lib.TIFFGetField(t, 257, ctypes.byref(h))
                n = w.value * h.value
                buf = (ctypes.c_uint32 * max(n, 1))()
                ok = lib.TIFFReadRGBAImageOriented(t, w.value, h.value, buf, 1, 0)
                res = struct.pack("<III", w.value, h.value, ok) + bytes(buf)[:n * 4]
            lib.TIFFClose(t)
        open(job["out"], "wb").write(res)
        log.append(1)
print(json.dumps(log))
'''


def wsl_path(p):
    p = str(Path(p).resolve()).replace("\\", "/")
    return "/mnt/" + p[0].lower() + p[2:]


def libtiff_run(jobs):
    r = it.wsl_python(LT_PY, json.dumps(jobs).encode(), timeout=1800)
    try:
        return json.loads(r.stdout.decode().strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        return None


def libtiff_read(paths):
    """[(path, page)] -> list of BGRA arrays as libtiff returns them (premultiplied when the
    image has alpha) or None."""
    jobs = [{"op": "read", "path": wsl_path(p), "page": pg, "out": wsl_path(str(p) + f".lt{pg}")}
            for p, pg in paths]
    libtiff_run(jobs)
    out = []
    for p, pg in paths:
        f = Path(str(p) + f".lt{pg}")
        if not f.exists():
            out.append(None)
            continue
        b = f.read_bytes()
        w, h, ok = struct.unpack_from("<III", b)
        f.unlink()
        out.append(np.frombuffer(b, np.uint8, count=w * h * 4, offset=12).reshape(h, w, 4)[..., [2, 1, 0, 3]].copy()
                   if ok and w and h else None)
    return out


def have_wsl_libtiff():
    r = it.wsl_python("import ctypes; ctypes.CDLL('libtiff.so.6'); print('ok')", b"", timeout=120)
    return r.returncode == 0 and b"ok" in r.stdout


# ================================================================== helpers

class TRunner(it.Runner):
    """Runner with a shorter timeout per batch (hang detection in the fuzz suite)."""

    def __init__(self, *a, timeout=900, **k):
        super().__init__(*a, **k)
        self.timeout = timeout

    def run(self, args, timeout=None):
        return super().run(args, timeout=self.timeout)


def dec_rec(data, name, exp, tol=0, flags=STRAIGHT, frame=0, meta=None, fmt=None):
    return Rec(it.OP_DECEX, 0, it.decex_input(data, flags=flags, frame=frame), it.expect_decex(exp, fmt=fmt, tol=tol,
                                                                                            meta=meta), name)


def err_rec(data, name, *codes, flags=STRAIGHT, frame=0, maxpx=0):
    return Rec(it.OP_DECEX, 0, it.decex_input(data, flags=flags, frame=frame, maxpx=maxpx), it.expect_err(*codes), name)


def imgopt(fmt=C.IMGF_TIFF, bitdepth=0, comp=0, dither=0, flags=0):
    """IMGOPT bytes (pointers 0)."""
    b = bytearray(C.IO_SIZE)
    struct.pack_into("<I", b, C.IO_FMT, fmt)
    struct.pack_into("<I", b, C.IO_FLAGS, flags)
    struct.pack_into("<I", b, C.IO_BITDEPTH, bitdepth)
    struct.pack_into("<I", b, C.IO_DITHER, dither)
    struct.pack_into("<I", b, C.IO_TIFFCOMP, comp)
    return bytes(b)


def rgb_of(bgra):
    return bgra[..., [2, 1, 0]]


def rand_img(r, w, h, kind="smooth"):
    """(h, w, 3) uint8 RGB test content: 'smooth' (gradients + noise, compresses well), 'noise'."""
    if kind == "noise":
        return r.integers(0, 256, (h, w, 3), dtype=np.uint8)
    yy, xx = np.mgrid[0:h, 0:w]
    base = np.stack([(xx * 255 // max(w - 1, 1)), (yy * 255 // max(h - 1, 1)), ((xx + yy) * 3) % 256], -1)
    return (base + r.integers(0, 8, (h, w, 3))).clip(0, 255).astype(np.uint8)


def parse_ifd(data):
    """Minimal classic-TIFF reader for the encoder checks: {tag: (type, [values])} of IFD0."""
    bo = "<" if data[:2] == b"II" else ">"
    off = struct.unpack_from(bo + "I", data, 4)[0]
    n = struct.unpack_from(bo + "H", data, off)[0]
    out = {}
    for i in range(n):
        tag, typ, cnt = struct.unpack_from(bo + "HHI", data, off + 2 + 12 * i)
        sz = TSZ.get(typ, 1) * cnt
        voff = off + 2 + 12 * i + 8 if sz <= 4 else struct.unpack_from(bo + "I", data, off + 2 + 12 * i + 8)[0]
        raw = data[voff:voff + sz]
        if typ in (3, 4):
            vals = list(struct.unpack_from(bo + TFMT[typ] * cnt, raw))
        elif typ == 5:
            vals = [struct.unpack_from(bo + "II", raw, 8 * k) for k in range(cnt)]
        else:
            vals = raw
        out[tag] = (typ, vals)
    nxt = struct.unpack_from(bo + "I", data, off + 2 + 12 * n)[0]
    return out, off, nxt


# ================================================================== suite: gen

def pick(r, seq):
    return seq[int(r.integers(len(seq)))]


def containers(r, sfmt, bps, spp, big_ok=True):
    """A few (comp, pred, planar, tile, rps, bo, big, fill) container variants for a pixel case."""
    out = [dict(comp=1, pred=1, planar=1, tile=None, rps=None, bo="<", big=False, fill=1)]
    comps = [5, 8, 32946, 32773, -5]
    for _ in range(2):
        comp = pick(r, comps)
        pred = 1
        if comp in (5, 8, 32946, -5):
            if sfmt == 3:
                pred = pick(r, [1, 2, 3]) if bps != 24 else pick(r, [1, 3])
            elif bps != 24:
                pred = pick(r, [1, 2])
        tile = pick(r, [None, None, (16, 16), (32, 16), (48, 32)])
        out.append(dict(comp=comp, pred=pred, planar=pick(r, [1, 2]) if spp > 1 else 1, tile=tile,
                        rps=None if tile else pick(r, [1, 3, 7, 16, None]),
                        bo=pick(r, ["<", ">"]), big=big_ok and r.random() < 0.3, fill=1))
    return out


def gen_cases(r, quick=False):
    """[(name, data, exp, tol, wic_tol or None, flags)]"""
    cases = []
    W, H = 37, 29

    def add(name, samples, bps, phot, sfmt=1, extra=None, alpha=None, assoc=False, cmap=None, wic=None,
            tol=0, spp_ctr=None, conts=None, tags=None):
        s = np.asarray(samples)
        exp = expected(s, bps, phot, sfmt, alpha, assoc, cmap)
        spp = s.shape[-1] if s.ndim == 3 else 1
        for ci, c in enumerate(conts or containers(r, sfmt, bps, spp)):
            data = make_tiff(s, bps, phot, sfmt, c["comp"], c["pred"], c["planar"], c["tile"], c["rps"],
                             c["bo"], c["big"], c["fill"], extra, tags, cmap)
            cn = (f"c{c['comp']} p{c['pred']} pl{c['planar']} {'t%dx%d' % c['tile'] if c['tile'] else 'r%s' % c['rps']}"
                  f" {'MM' if c['bo'] == '>' else 'II'}{' big' if c['big'] else ''}{' fill2' if c['fill'] == 2 else ''}")
            cases.append((f"{name} [{cn}]", data, exp, tol, wic if ci == 0 else None, STRAIGHT))

    def ints(bps, shape):
        return r.integers(0, 1 << bps, shape, dtype=np.int64) if bps < 63 else None
    # grey, every depth, WhiteIsZero / BlackIsZero
    for bps in (1, 2, 3, 4, 6, 8, 10, 12, 14, 16, 24, 32):
        for phot in (0, 1):
            v = ints(bps, (H, W))
            v[0, :min(W, 1 << bps)] = np.arange(min(W, 1 << bps))
            wt = {1: 0, 2: 0, 4: 0, 6: 0, 8: 0, 16: 1, 12: 1}.get(bps)
            add(f"grey {bps}-bit phot {phot}", v, bps, phot, wic=wt)
    for bps in (8, 16, 32):
        add(f"grey signed {bps}-bit", ints(bps, (H, W)), bps, 1, sfmt=2)
    # grey + alpha
    for bps in (8, 16):
        for ex, al, asc, wt in (([2], 1, False, 0), ([1], 1, True, 1), ([0], 1, False, None), (None, 1, False, 0)):
            v = ints(bps, (H, W, 2))
            if asc:
                v[..., 0] = np.minimum(v[..., 0], v[..., 1])
            add(f"grey+alpha {bps}-bit extra {ex}", v, bps, 1, extra=ex, alpha=al, assoc=asc,
                wic=wt if bps == 8 else None)
    # RGB / RGBA
    for bps in (8, 16, 32, 5, 12):
        wt = 0 if bps in (8, 16) else None
        add(f"rgb {bps}-bit", ints(bps, (H, W, 3)), bps, 2, wic=wt)
        for ex, asc, wt2 in (([2], False, 0), ([1], True, 1), ([0], False, 0), (None, False, 0)):
            v = ints(bps, (H, W, 4))
            if asc:
                v[..., :3] = np.minimum(v[..., :3], v[..., 3:4])
            add(f"rgba {bps}-bit extra {ex}", v, bps, 2, extra=ex, alpha=3, assoc=asc,
                wic=wt2 if bps in (8, 16) else None)
    v = ints(8, (H, W, 5))
    add("rgb + alpha + extra (spp 5)", v, 8, 2, extra=[2, 0], alpha=3)
    add("rgb + 2 non-alpha extras (spp 5)", v, 8, 2, extra=[0, 0])
    add("rgb + unspecified extra (spp 4, ExtraSamples 3)", ints(8, (H, W, 4)), 8, 2, extra=[3])
    # floats
    for bps in (16, 24, 32):
        f = r.uniform(-0.2, 1.3, (H, W, 3)).astype(np.float32)
        f[0, :4, 0] = [np.nan, np.inf, -np.inf, 0.5]
        if bps == 24:
            f = f24_round(f)
        wt = 1 if bps == 32 else None                   # WIC: half grey read as integers, half
        add(f"rgb float{bps}", f, bps, 2, sfmt=3, wic=wt)  # alpha sRGB-encoded, half +inf -> 0
        g = r.uniform(0, 1, (H, W)).astype(np.float32)
        if bps == 24:
            g = f24_round(g)
        add(f"grey float{bps}", g, bps, 1, sfmt=3, wic=wt)
        fa = r.uniform(0, 1, (H, W, 4)).astype(np.float32)
        if bps == 24:
            fa = f24_round(fa)
        add(f"rgba float{bps} unassociated", fa, bps, 2, sfmt=3, extra=[2], alpha=3, wic=wt)
        fp = fa.copy()
        fp[..., :3] = np.minimum(fp[..., :3], fp[..., 3:4])
        add(f"rgba float{bps} associated", fp, bps, 2, sfmt=3, extra=[1], alpha=3, assoc=True)
    # palette
    for bps in (1, 2, 4, 8, 16):
        n = 1 << bps
        cm = r.integers(0, 65536, 3 * n)
        add(f"palette {bps}-bit", ints(bps, (H, W)), bps, 3, cmap=cm, wic=0 if bps <= 8 else None)
    cm = r.integers(0, 65536, 3 * 16)
    add("palette 8-bit, 16-entry ColorMap", ints(4, (H, W)) * 17, 8, 3, cmap=cm)
    add("palette + alpha", np.stack([ints(8, (H, W)), ints(8, (H, W))], -1), 8, 3, extra=[2], alpha=1,
        cmap=r.integers(0, 65536, 768))
    # CMYK
    for bps in (8, 16):
        add(f"cmyk {bps}-bit", ints(bps, (H, W, 4)), bps, 5)
        add(f"cmyk+alpha {bps}-bit", ints(bps, (H, W, 5)), bps, 5, extra=[2], alpha=4)
    # YCbCr 1x1 (default ReferenceBlackWhite and custom coefficients / range)
    add("ycbcr 1x1", ints(8, (H, W, 3)), 8, 6, tags={530: (3, [1, 1])})
    v = ints(8, (H, W, 3))
    exp = expected(v, 8, 6, luma=(0.2126, 0.7152, 0.0722), rbw=(16, 235, 128, 240, 128, 240))
    data = make_tiff(v, 8, 6, tags={530: (3, [1, 1]), 529: (5, [(2126, 10000), (7152, 10000), (722, 10000)]),
                                     532: (5, [(16, 1), (235, 1), (128, 1), (240, 1), (128, 1), (240, 1)])})
    cases.append(("ycbcr 1x1 BT.709 coefficients, video range", data, exp, 0, None, STRAIGHT))
    # CIELab
    for bps in (8, 16):
        v = ints(bps, (H, W, 3))
        add(f"cielab {bps}-bit", v, bps, 8)
    return cases


def ysub_cases(r):
    out = []
    for (hh, vv) in ((2, 2), (2, 1), (1, 2), (4, 2), (4, 4), (4, 1), (1, 1)):
        for (W, H) in ((37, 29), (16, 16), (5, 3)):
            nbx, nby = -(-W // hh), -(-H // vv)
            Y = r.integers(0, 256, (H, W))
            Cb = r.integers(0, 256, (nby, nbx))
            Cr = r.integers(0, 256, (nby, nbx))
            exp = ysub_expected(Y, Cb, Cr, (hh, vv))
            for comp, rps, tile in ((1, None, None), (5, 4 * vv, None), (32773, vv, None), (8, None, (16, 16))):
                if tile and (tile[0] % hh or tile[1] % vv):
                    continue
                data = ycc_sub_tiff(Y, Cb, Cr, (hh, vv), comp=comp, rps=rps, tile=tile)
                out.append((f"ycbcr {hh}x{vv} {W}x{H} c{comp} r{rps} t{tile}", data, exp, 0, None, STRAIGHT))
    # default subsampling (tag missing = 2 x 2)
    Y = r.integers(0, 256, (6, 8))
    Cb, Cr = r.integers(0, 256, (3, 4)), r.integers(0, 256, (3, 4))
    data = ycc_sub_tiff(Y, Cb, Cr, (2, 2), tags={530: None})
    out.append(("ycbcr without YCbCrSubsampling (default 2x2)", data, ysub_expected(Y, Cb, Cr, (2, 2)), 0, None,
                STRAIGHT))
    return out


def layout_cases(r):
    """Special layouts: FillOrder 2, BigTIFF / LONG8 / SHORT offsets, strip padding, rps 0,
    missing byte counts, pages."""
    out = []
    W, H = 33, 21
    rgb = r.integers(0, 256, (H, W, 3))
    exp = expected(rgb, 8, 2)
    for comp in (1, 5, 8, 32773, -5):
        out.append((f"FillOrder 2 comp {comp}", make_tiff(rgb, 8, 2, comp=comp, fill=2, rps=5), exp, 0, None, STRAIGHT))
    for comp in (1, 5):
        out.append((f"BigTIFF LONG8 offsets comp {comp}", make_tiff(rgb, 8, 2, comp=comp, big=True, rps=4, offtype=16),
                    exp, 0, None, STRAIGHT))
        out.append((f"BigTIFF LONG offsets comp {comp}", make_tiff(rgb, 8, 2, comp=comp, big=True, rps=4, offtype=4),
                    exp, 0, None, STRAIGHT))
    small = rgb[:5, :5]
    out.append(("SHORT strip offsets", make_tiff(small, 8, 2, rps=2, offtype=3), expected(small, 8, 2), 0, None, STRAIGHT))
    # last strip holds full RowsPerStrip rows (LZW / Deflate / none / PackBits)
    for comp in (1, 5, 8, 32773):
        pad = np.zeros((24, W, 3), np.int64)
        pad[:H] = rgb
        pg = make_page(pad, 8, 2, comp=comp, rps=8)
        pg["tags"][257] = (4, [H])
        out.append((f"last strip padded to RowsPerStrip comp {comp}", write_tiff([pg]), exp, 0, None, STRAIGHT))
    # RowsPerStrip 0 and missing: whole image; missing StripByteCounts (uncompressed / LZW)
    out.append(("RowsPerStrip 0", make_tiff(rgb, 8, 2, tags={278: (4, [0])}), exp, 0, None, STRAIGHT))
    out.append(("RowsPerStrip missing", make_tiff(rgb, 8, 2, tags={278: None}), exp, 0, None, STRAIGHT))
    for comp in (1, 5):
        pg = make_page(rgb, 8, 2, comp=comp)
        pg["override"] = {279: None}
        out.append((f"no StripByteCounts comp {comp}", write_tiff([pg]), exp, 0, None, STRAIGHT))
    # sparse strip (byte count 0) -> zeros
    pg = make_page(rgb, 8, 2, rps=7)
    d = write_tiff([pg])
    tags, _, _ = parse_ifd(d)
    cnts = list(tags[279][1])
    cnts[1] = 0
    pg["override"] = {279: (4, cnts)}
    e2 = exp.copy()
    e2[7:14] = [0, 0, 0, 255]
    out.append(("sparse strip (byte count 0)", write_tiff([pg]), e2, 0, None, STRAIGHT))
    # photometric missing: RGB for 3 samples, BlackIsZero for 1
    out.append(("Photometric missing (3 samples)", make_tiff(rgb, 8, 2, tags={262: None}), exp, 0, None, STRAIGHT))
    g = r.integers(0, 256, (H, W))
    out.append(("Photometric missing (1 sample)", make_tiff(g, 8, 1, tags={262: None}), expected(g, 8, 1), 0, None,
                STRAIGHT))
    # SampleFormat 4 (void) = unsigned, BitsPerSample with one value for 3 samples
    out.append(("SampleFormat void", make_tiff(rgb, 8, 2, tags={339: (3, [4, 4, 4])}), exp, 0, None, STRAIGHT))
    out.append(("BitsPerSample single value", make_tiff(rgb, 8, 2, tags={258: (3, [8])}), exp, 0, None, STRAIGHT))
    # predictor with PackBits / none is ignored (libtiff)
    out.append(("Predictor 2 with no compression (ignored)", make_tiff(rgb, 8, 2, tags={317: (3, [2])}), exp, 0,
                None, STRAIGHT))
    # 16-bit predictor 2, big endian; 32-bit predictor 2; bit-packed predictor 2
    for bps, bo in ((16, ">"), (16, "<"), (32, ">"), (4, "<"), (12, ">"), (1, "<")):
        v = r.integers(0, 1 << bps, (H, W, 3 if bps != 1 else 1), dtype=np.int64)
        ph = 2 if bps != 1 else 1
        out.append((f"predictor 2 {bps}-bit {bo}", make_tiff(v, bps, ph, comp=5, pred=2, bo=bo, rps=6),
                    expected(v, bps, ph), 0, None, STRAIGHT))
    return out


def run_gen(runner, suite, r, work, ext):
    cases = gen_cases(r) + ysub_cases(r) + layout_cases(r)
    recs = [dec_rec(d, n, e, tol) for (n, d, e, tol, _, fl) in cases]
    it.run_suite(runner, suite, recs)
    # PARGB output (no IMGD_STRAIGHT) for a sample of cases: premultiplied expectation
    recs = []
    for (n, d, e, tol, _, fl) in cases[::7]:
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=0),
                        it.expect_decex(it.premul(e), tol=tol + (1 if tol else 0)), n + " (PARGB)"))
    it.run_suite(runner, suite, recs)
    if not ext or runner.target != "win":
        return
    # cross-check of the model with WIC (the decoder paint.net uses)
    sel = [(n, d, e, wt) for (n, d, e, tol, wt, fl) in cases if wt is not None]
    paths = []
    for i, (n, d, e, wt) in enumerate(sel):
        p = Path(work) / f"gen_wic_{i}.tif"
        p.write_bytes(d)
        paths.append((p, 0))
    res = wic_decode_batch(work, paths)
    for (n, d, e, wt), rr in zip(sel, res):
        if rr is None:
            suite.fail(f"WIC could not decode {n}")
            continue
        m = it.max_abs(rr[3], e)
        suite.check(m <= wt, f"model vs WIC {n}: max |d| {m} > {wt}")
    suite.notes.append(f"model cross-checked with WIC on {len(sel)} files")


# ================================================================== suite: wic

WIC_FORMATS = ["BlackWhite", "Gray8", "Gray16", "Bgra32", "Pbgra32", "Rgba64", "Prgba64", "Cmyk32", "Bgr24",
               "Rgb48", "Indexed8"]
WIC_COMPS = ["None", "Ccitt3", "Ccitt4", "Lzw", "Rle", "Zip"]


def run_wic(runner, suite, r, work, lt_ok):
    jobs, meta = [], []
    src = it.rand_bgra(random.Random(int(r.integers(1 << 30))), 45, 31, "mixed")
    src[..., :3] = np.maximum(src[..., :3], 0)
    for pf in WIC_FORMATS:
        for comp in WIC_COMPS:
            if comp.startswith("Ccitt") and pf != "BlackWhite":
                continue
            if pf == "Indexed8":
                continue                                # needs a palette: skipped
            out = Path(work) / f"wic_{pf}_{comp}.tif"
            jobs.append((src, out, pf, comp))
            meta.append((pf, comp, out))
    ok = wic_encode_batch(work, jobs)
    have = [(pf, comp, out) for (pf, comp, out), o in zip(meta, ok) if o]
    missing = [f"{pf}/{comp}" for (pf, comp, out), o in zip(meta, ok) if not o]
    if missing:
        suite.notes.append("WIC could not write: " + ", ".join(missing))
    ref = wic_decode_batch(work, [(out, 0) for (_, _, out) in have])
    lt = libtiff_read([(out, 0) for (_, _, out) in have]) if lt_ok else [None] * len(have)
    recs = []
    for (pf, comp, out), rw, rl in zip(have, ref, lt):
        data = out.read_bytes()
        name = f"WIC {pf} {comp}"
        if pf == "Cmyk32":
            if rl is None:
                suite.notes.append(f"{name}: no libtiff reference")
                continue
            recs.append(dec_rec(data, name + " (vs libtiff)", rl))
            continue
        if rw is None:
            suite.fail(f"{name}: WIC decode failed")
            continue
        tol = 1 if pf in ("Gray16", "Pbgra32", "Prgba64") else 0
        recs.append(dec_rec(data, name, rw[3], tol=tol))
    it.run_suite(runner, suite, recs)


# ================================================================== suite: ffmpeg

FF_PIX = ["rgb24", "rgba", "rgb48le", "rgba64le", "gray", "gray16le", "ya8", "ya16le", "pal8", "monob", "monow",
          "yuv420p", "yuv422p", "yuv444p", "yuv410p", "yuv411p", "yuv440p"]
FF_COMP = ["raw", "packbits", "lzw", "deflate"]


def run_ffmpeg(runner, suite, r, work, lt_ok):
    if not it.have_exe("ffmpeg"):
        suite.notes.append("ffmpeg not found: skipped")
        return
    src = it.rand_bgra(random.Random(int(r.integers(1 << 30))), 42, 30, "random")
    yy, xx = np.mgrid[0:30, 0:42]
    src[..., 0] = (xx * 6).clip(0, 255)
    src[..., 1] = (yy * 8).clip(0, 255)
    files = []
    for pix in FF_PIX:
        for comp in FF_COMP:
            out = Path(work) / f"ff_{pix}_{comp}.tif"
            out.unlink(missing_ok=True)
            if it.ffmpeg_encode(src, out, ["-pix_fmt", pix, "-compression_algo", comp, "-dpi", "300"]):
                files.append((pix, comp, out))
            else:
                suite.notes.append(f"ffmpeg -pix_fmt {pix} -compression_algo {comp}: not written")
    wref = wic_decode_batch(work, [(o, 0) for (_, _, o) in files])
    lref = libtiff_read([(o, 0) for (_, _, o) in files]) if lt_ok else [None] * len(files)
    recs = []
    for (pix, comp, out), rw, rl in zip(files, wref, lref):
        data = out.read_bytes()
        name = f"ffmpeg {pix} {comp}"
        dpi = {"dpix": 300 << 16, "dpiy": 300 << 16, "resunit": C.IMR_INCH}
        if pix.startswith("yuv"):
            # reference: the blocks of the uncompressed file (libtiff's TIFFRGBAImage mishandles
            # partial 4 x 4 blocks); where the width is a block multiple libtiff must agree
            model = ysub_model_from_file((Path(work) / f"ff_{pix}_raw.tif").read_bytes())
            if rl is not None and pix not in ("yuv410p", "yuv411p"):
                suite.check(it.max_abs(rl, model) == 0, f"{name}: model vs libtiff {it.max_abs(rl, model)}")
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT),
                            it.expect_decex(model, meta=dpi), name + " (vs block model)"))
            continue
        if rw is None:
            suite.fail(f"{name}: WIC could not decode")
            continue
        tol = 1 if pix in ("gray16le", "ya16le") else 0
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT),
                        it.expect_decex(rw[3], tol=tol, meta=dpi), name))
    it.run_suite(runner, suite, recs)


# ================================================================== suite: libtiff

def lt_page(tags, data, chunk, tiled=False):
    return {"tags": [[k, v] for k, v in tags], "data": bytes(data).hex(), "chunk": chunk, "tiled": tiled}


def run_libtiff(runner, suite, r, work, lt_ok):
    if not lt_ok:
        suite.notes.append("libtiff (WSL) not available: skipped")
        return
    jobs, names = [], []

    model = {}

    def job(name, pages, exp=None):
        p = Path(work) / f"lt_{len(names)}.tif"
        p.unlink(missing_ok=True)
        jobs.append({"op": "write", "path": wsl_path(p), "pages": pages})
        names.append((name, p))
        if exp is not None:
            model[name] = exp
    # bilevel content with long runs and fine detail
    for (w, h) in ((203, 97), (1, 1), (64, 5), (1728, 12)):
        bl = np.zeros((h, w), np.uint8)
        for _ in range(max(1, w * h // 400)):
            x0, y0 = r.integers(0, w), r.integers(0, h)
            bl[y0:y0 + r.integers(1, 20), x0:x0 + r.integers(1, 60)] ^= 1
        if w > 10:
            bl[0, ::2] = 1
            bl[-1, :] = 1
        rb = (w + 7) // 8
        rows = np.packbits(bl, axis=1)
        for rps in (h, 5):
            rps = min(rps, h)
            raw = b"".join(rows[y:y + rps].tobytes().ljust(rb * rps, b"\0") for y in range(0, h, rps))
            for comp, name, extra in ((2, "RLE", []), (3, "T.4 1D", [[292, 0]]), (3, "T.4 2D", [[292, 1]]),
                                      (3, "T.4 2D fill", [[292, 5]]), (3, "T.4 1D fill", [[292, 4]]), (4, "T.6", [])):
                for fill in (1, 2):
                    for phot in (0, 1):
                        tags = [[256, w], [257, h], [258, 1], [277, 1], [259, comp], [262, phot], [266, fill],
                                [278, rps], [284, 1]] + extra
                        job(f"CCITT {name} {w}x{h} rps {rps} fill {fill} phot {phot}",
                            [lt_page([tuple(t) for t in tags], raw, rb * rps)])
    # JPEG
    img = np.cumsum(r.integers(0, 12, (64, 80, 3)), axis=1).clip(0, 255).astype(np.uint8)
    for sub in ((2, 2), (2, 1), (1, 1)):
        job(f"JPEG YCbCr {sub[0]}x{sub[1]}",
            [lt_page([(256, 80), (257, 64), (258, 8), (277, 3), (259, 7), (262, 6), (65538, 1), (530, list(sub)),
                      (278, 16), (284, 1)], img.tobytes(), 80 * 3 * 16)])
    job("JPEG YCbCr 2x2, odd size", [lt_page([(256, 77), (257, 61), (258, 8), (277, 3), (259, 7), (262, 6),
                                             (65538, 1), (530, [2, 2]), (278, 16), (284, 1)],
                                            img[:61, :77].tobytes().ljust(77 * 3 * 64, b"\0"), 77 * 3 * 16)])
    job("JPEG RGB", [lt_page([(256, 80), (257, 64), (258, 8), (277, 3), (259, 7), (262, 2), (278, 32), (284, 1)],
                             img.tobytes(), 80 * 3 * 32)])
    job("JPEG grey", [lt_page([(256, 80), (257, 64), (258, 8), (277, 1), (259, 7), (262, 1), (278, 64), (284, 1)],
                              img[..., 1].tobytes(), 80 * 64)])
    job("JPEG quality 30", [lt_page([(256, 80), (257, 64), (258, 8), (277, 3), (259, 7), (262, 2), (65537, 30),
                                    (278, 64), (284, 1)], img.tobytes(), 80 * 3 * 64)])
    tiles = b"".join(np.pad(img, ((0, 0), (0, 16), (0, 0)))[ty * 32:(ty + 1) * 32, tx * 32:(tx + 1) * 32].tobytes()
                     for ty in range(2) for tx in range(3))
    job("JPEG tiles YCbCr", [lt_page([(256, 80), (257, 64), (258, 8), (277, 3), (259, 7), (262, 6), (65538, 1),
                                      (530, [2, 2]), (322, 32), (323, 32), (284, 1)], tiles, 32 * 32 * 3, True)])
    # predictor 2 16 bit, predictor 3 float, planar, CMYK, palette
    v16 = r.integers(0, 65536, (33, 45, 3)).astype("<u2")
    for comp in (5, 8):
        job(f"libtiff 16-bit predictor 2 comp {comp}",
            [lt_page([(256, 45), (257, 33), (258, 16), (277, 3), (259, comp), (262, 2), (317, 2), (278, 8), (284, 1)],
                     v16.tobytes(), 45 * 6 * 8)], exp=expected(v16.astype(np.int64), 16, 2))
    cmyk = r.integers(0, 256, (33, 45, 4)).astype(np.uint8)
    job("libtiff CMYK LZW", [lt_page([(256, 45), (257, 33), (258, 8), (277, 4), (259, 5), (262, 5), (278, 11), (284, 1)],
                                     cmyk.tobytes(), 45 * 4 * 11)])
    planar = np.ascontiguousarray(img[:33, :45].transpose(2, 0, 1))
    job("libtiff planar RGB Deflate", [lt_page([(256, 45), (257, 33), (258, 8), (277, 3), (259, 8), (262, 2),
                                                (278, 33), (284, 2)], planar.tobytes(), 45 * 33)])
    idx = r.integers(0, 16, (33, 45)).astype(np.uint8)
    cm = [int(x) for x in r.integers(0, 65536, 48)]
    pk = b"".join(np.packbits(np.unpackbits(idx[y][:, None], axis=1)[:, 4:].reshape(-1)).tobytes() for y in range(33))
    job("libtiff 4-bit palette PackBits", [lt_page([(256, 45), (257, 33), (258, 4), (277, 1), (259, 32773), (262, 3),
                                                    (320, cm), (278, 33), (284, 1)], pk, len(pk))],
        exp=expected(idx.astype(np.int64), 4, 3, cmap=cm))
    log = libtiff_run(jobs)
    if not log:
        suite.fail("libtiff writer failed")
        return
    written = [(n, p) for (n, p), ok in zip(names, log) if ok and p.exists()]
    for (n, p), ok in zip(names, log):
        if not ok:
            suite.notes.append(f"libtiff could not write {n}")
    refs = libtiff_read([(p, 0) for (_, p) in written])
    recs = []
    for (n, p), ref in zip(written, refs):
        if ref is None:
            suite.fail(f"{n}: libtiff cannot read its own file")
            continue
        if n in model:                          # libtiff: 16 bit round(v / 257), palette v >> 8
            suite.check(it.max_abs(ref, model[n]) <= 1, f"{n}: model vs libtiff > 1")
            ref = model[n]
        recs.append(dec_rec(p.read_bytes(), n, ref))
    it.run_suite(runner, suite, recs)


# ================================================================== suite: encode

def enc_images(r):
    """(name, straight BGRA, surface format) test images for the encoder."""
    out = []
    op = np.zeros((70, 93, 4), np.uint8)
    op[..., :3] = rand_img(r, 93, 70)[..., ::-1]
    op[..., 3] = 255
    out.append(("opaque smooth", op, C.FMT_ARGB))
    nz = np.concatenate([r.integers(0, 256, (41, 57, 3), dtype=np.uint8), np.full((41, 57, 1), 255, np.uint8)], -1)
    out.append(("opaque noise", nz, C.FMT_ARGB))
    al = it.rand_bgra(random.Random(int(r.integers(1 << 30))), 47, 33, "random")
    out.append(("alpha random", al, C.FMT_ARGB))
    mx = it.rand_bgra(random.Random(int(r.integers(1 << 30))), 29, 23, "mixed")
    out.append(("alpha mixed (PARGB input)", mx, C.FMT_PARGB))
    few = np.zeros((40, 52, 4), np.uint8)
    pal = r.integers(0, 256, (37, 3))
    ii = r.integers(0, 37, (40, 52))
    few[..., :3] = pal[ii][..., ::-1]
    few[..., 3] = 255
    out.append(("37 colours", few, C.FMT_ARGB))
    xr = op.copy()
    out.append(("XRGB input", xr, C.FMT_XRGB))
    big = np.zeros((300, 257, 4), np.uint8)       # several strips
    big[..., :3] = rand_img(r, 257, 300)[..., ::-1]
    big[..., 3] = 255
    out.append(("300x257 (strips)", big, C.FMT_ARGB))
    one = np.array([[[10, 20, 30, 255]]], np.uint8)
    out.append(("1x1", one, C.FMT_ARGB))
    return out


def surf_bytes(px, fmt):
    if fmt == C.FMT_PARGB:
        return it.surface_input(it.premul(px), fmt)
    if fmt == C.FMT_XRGB:
        q = px.copy()
        q[..., 3] = 0
        return it.surface_input(q, fmt)
    return it.surface_input(px, fmt)


def over_white(px):
    p = px.astype(np.int64)
    a = p[..., 3:4]
    c = (p[..., :3] * a + 255 * (255 - a) + 127) // 255
    return np.concatenate([c, np.full_like(a, 255)], -1).astype(np.uint8)


def enc_expect(px, fmt, bd):
    """What decoding our file must give (None = quantized: checked by PSNR)."""
    src = it.unpremul(it.premul(px)) if fmt == C.FMT_PARGB else px.copy()
    if fmt == C.FMT_XRGB:
        src[..., 3] = 255
    opaque = bool((src[..., 3] == 255).all())
    ncol = len(np.unique(src.reshape(-1, 4).view(np.uint32)))
    if bd == 32 or (bd == 0 and not opaque):
        return src, 32
    if bd == 24 or (bd == 0 and ncol > 256):
        return over_white(src), 24
    if bd == 0:
        return src, 8
    exact = ncol <= 256 and opaque
    return (over_white(src) if exact else None), 8


def run_encode(runner, suite, r, work, ext, lt_ok):
    imgs = enc_images(r)
    recs, files = [], []
    comps = [(0, 5, "default"), (C.TIFC_NONE, 1, "none"), (C.TIFC_LZW, 5, "LZW"), (C.TIFC_DEFLATE, 8, "Deflate"),
             (C.TIFC_PACKBITS, 32773, "PackBits")]
    for (iname, px, fmt) in imgs:
        for bd in (0, 32, 24, 8):
            for (tc, tval, cname) in comps:
                exp, rbd = enc_expect(px, fmt, bd)
                name = f"encode {iname} bd {bd} {cname}"
                data = imgopt(bitdepth=bd, comp=tc) + surf_bytes(px, fmt)

                def chk(res, out, exp=exp, rbd=rbd, tval=tval, name=name, px=px):
                    if res != 0:
                        return f"error {it.err_name(res)}"
                    f = bytes(out)
                    if f[:4] != b"II*\0":
                        return "not a little-endian TIFF"
                    tags, ifd, nxt = parse_ifd(f)
                    if ifd & 1 or nxt != 0:
                        return f"IFD at {ifd}, next {nxt}"
                    if tags[259][1][0] != tval:
                        return f"compression {tags[259][1][0]} != {tval}"
                    spp = tags[277][1][0]
                    if {32: 4, 24: 3, 8: 1}[rbd] != spp:
                        return f"samples {spp} for {rbd} bit"
                    if rbd == 32 and tags.get(338, (0, [0]))[1][0] != 2:
                        return "ExtraSamples != 2"
                    if rbd == 8 and (tags[262][1][0] != 3 or len(tags[320][1]) != 768):
                        return "palette tags"
                    pred = tags.get(317, (0, [1]))[1][0]
                    if pred != (2 if tval in (5, 8) and rbd != 8 else 1):
                        return f"predictor {pred}"
                    rowb = tags[256][1][0] * spp
                    rps = tags[278][1][0]
                    if rps != min(tags[257][1][0], max(1, 65536 // rowb)):
                        return f"RowsPerStrip {rps}"
                    files.append((name, f, exp, rbd))
                    return None
                recs.append(Rec(it.OP_ENCODE, 0, data, chk, name))
    it.run_suite(runner, suite, recs)
    # decode our files with our decoder: exact (quantized 8 bit: PSNR)
    recs = []
    for (name, f, exp, rbd) in files:
        if exp is None:
            continue
        recs.append(dec_rec(f, name + " -> decode", exp))
    it.run_suite(runner, suite, recs)
    # dithering levels (8 bit, many colours): valid file, PSNR vs the composite over white
    recs = []
    px = imgs[0][1]
    for dl in range(0, 10):
        def chk(res, out, px=px, dl=dl):
            return None if res == 0 and bytes(out)[:4] == b"II*\0" else f"error {it.err_name(res)}"
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(bitdepth=8, dither=dl) + surf_bytes(px, C.FMT_ARGB), chk,
                        f"8 bit dither {dl - 1 if dl else 'default'}"))
    res = it.run_suite(runner, suite, recs)
    recs = []
    for dl, rr in zip(range(10), res):
        if rr and rr[0] == 0:
            def pchk(res2, out2, px=px):
                if res2 != 0:
                    return f"error {it.err_name(res2)}"
                d = it.parse_decex(out2)
                p = it.psnr(d["px"][..., :3], over_white(px)[..., :3])
                return None if p > 28 else f"PSNR {p:.1f}"
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(bytes(rr[1]), flags=STRAIGHT), pchk,
                            f"8 bit dither {dl} decode PSNR"))
    it.run_suite(runner, suite, recs)
    # metadata through IO_META (op 61): ICC, XMP, dpi (inch / cm)
    icc = bytes(it_icc_profile())
    xmp = b'<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"/><?xpacket end="w"?>'
    recs = []
    metas = []
    for (dpix, dpiy, unit, flags, name) in (
            (300 << 16, 300 << 16, C.IMR_INCH, C.IOF_EMBED_ICC | C.IOF_EMBED_XMP, "300 dpi ICC XMP"),
            ((72 << 16) + 32768, 150 << 16, C.IMR_INCH, C.IOF_EMBED_ICC, "72.5 x 150 dpi ICC"),
            (254 << 16, 254 << 16, C.IMR_CM, C.IOF_EMBED_XMP, "100 dpcm XMP"),
            (0, 0, 0, 0, "no dpi (96)")):
        hdr = struct.pack("<6I", dpix, dpiy, unit, len(icc), len(xmp), 0)
        data = imgopt(flags=flags) + hdr + icc + xmp + surf_bytes(imgs[0][1], C.FMT_ARGB)
        rr = runner.one(OP_ENCMETA, 0, data)
        if rr[0] != 0:
            suite.fail(f"op 61 {name}: {it.err_name(rr[0])}")
            continue
        f = bytes(rr[1])
        tags, _, _ = parse_ifd(f)
        exp_dpi = (dpix or 96 << 16, dpiy or dpix or 96 << 16)
        mexp = {"dpix": exp_dpi[0], "dpiy": exp_dpi[1], "resunit": C.IMR_CM if unit == C.IMR_CM else C.IMR_INCH}
        if unit == C.IMR_CM:
            mexp = {"resunit": C.IMR_CM}
            num, den = tags[282][1][0]
            dpc = num / den
            suite.check(abs(dpc * 2.54 - 254) < 0.01, f"{name}: XResolution {num}/{den} per cm")
        want_icc = icc if flags & C.IOF_EMBED_ICC else b""
        want_xmp = xmp if flags & C.IOF_EMBED_XMP else b""
        suite.check((34675 in tags) == bool(want_icc), f"{name}: ICC tag presence")
        suite.check((700 in tags) == bool(want_xmp), f"{name}: XMP tag presence")

        def mchk(res2, out2, want_icc=want_icc, want_xmp=want_xmp, mexp=mexp):
            if res2 != 0:
                return f"error {it.err_name(res2)}"
            d = it.parse_decex(out2)
            if d["icc"] != want_icc:
                return f"ICC {len(d['icc'])} bytes, expected {len(want_icc)}"
            if d["xmp"] != want_xmp:
                return "XMP differs"
            for k, v in mexp.items():
                if d["meta"][k] != v:
                    return f"meta {k} = {d['meta'][k]}, expected {v}"
            return None
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(f, flags=STRAIGHT | C.IMGD_ICC | C.IMGD_XMP), mchk,
                        f"meta round trip {name}"))
        metas.append((name, f))
    it.run_suite(runner, suite, recs)
    if not ext or runner.target != "win":
        return
    # external decoders: WIC, ffmpeg, libtiff
    sel = [(n, f, e, rbd) for (n, f, e, rbd) in files if e is not None]
    paths = []
    for i, (n, f, e, rbd) in enumerate(sel):
        p = Path(work) / f"enc_{i}.tif"
        p.write_bytes(f)
        paths.append((p, 0))
    wres = wic_decode_batch(work, paths)
    for (n, f, e, rbd), rr in zip(sel, wres):
        if rr is None:
            suite.fail(f"WIC cannot read {n}")
            continue
        suite.check(it.max_abs(rr[3], e) == 0, f"WIC {n}: max |d| {it.max_abs(rr[3], e)}")
    if it.have_exe("ffmpeg"):
        bad = 0
        for (n, f, e, rbd), (p, _) in zip(sel, paths):
            ff = it.ffmpeg_decode(p, "bgra")
            if ff is None:
                suite.fail(f"ffmpeg cannot read {n}")
                bad += 1
                continue
            suite.check(it.max_abs(ff, e) == 0, f"ffmpeg {n}: max |d| {it.max_abs(ff, e)}")
    if lt_ok:
        lres = libtiff_read(paths)
        for (n, f, e, rbd), rl in zip(sel, lres):
            if rl is None:
                suite.fail(f"libtiff cannot read {n}")
                continue
            suite.check(it.max_abs(rl, it.premul(e)) == 0, f"libtiff {n}: max |d| {it.max_abs(rl, it.premul(e))}")
    suite.notes.append(f"{len(sel)} encoded files verified with WIC / ffmpeg / libtiff")


def it_icc_profile():
    """A small valid ICC v2 header + empty tag table (icc_check accepts it)."""
    b = bytearray(132)
    struct.pack_into(">I", b, 0, 132)
    b[4:8] = b"ASMG"
    struct.pack_into(">I", b, 8, 0x02100000)
    b[12:16] = b"mntr"
    b[16:20] = b"RGB "
    b[20:24] = b"XYZ "
    b[36:40] = b"acsp"
    struct.pack_into(">I", b, 128, 0)
    return bytes(b)


# ================================================================== suite: meta

def run_meta(runner, suite, r, work):
    W, H = 23, 17
    rgb = r.integers(0, 256, (H, W, 3))
    exp = expected(rgb, 8, 2)
    recs = []
    # IMGMETA fields
    for (name, data, m) in (
            ("rgb8 meta", make_tiff(rgb, 8, 2), dict(w=W, h=H, bits=8, model=C.IMM_RGB, channels=3, frames=1,
                                                      flags=C.IMF_LOSSLESS, orient=1)),
            ("rgba16 assoc meta", make_tiff(np.minimum(r.integers(0, 65536, (H, W, 4)), 65535), 16, 2, extra=[1]),
             dict(bits=16, channels=4, flags=C.IMF_LOSSLESS | C.IMF_ALPHA | C.IMF_SRC_PREMUL | C.IMF_HDR)),
            ("float meta", make_tiff(r.uniform(0, 1, (H, W, 3)).astype(np.float32), 32, 2, sfmt=3),
             dict(bits=32, flags=C.IMF_LOSSLESS | C.IMF_FLOAT | C.IMF_HDR)),
            ("cmyk meta", make_tiff(r.integers(0, 256, (H, W, 4)), 8, 5), dict(model=C.IMM_CMYK, channels=4)),
            ("palette meta", make_tiff(r.integers(0, 16, (H, W)), 4, 3, cmap=r.integers(0, 65536, 48)),
             dict(model=C.IMM_PAL, bits=4)),
            ("lab meta", make_tiff(r.integers(0, 256, (H, W, 3)), 8, 8), dict(model=C.IMM_LAB)),
            ("ycc meta", make_tiff(r.integers(0, 256, (H, W, 3)), 8, 6, tags={530: (3, [1, 1])}), dict(model=C.IMM_YCC)),
            ("grey meta", make_tiff(r.integers(0, 2, (H, W)), 1, 0), dict(model=C.IMM_GRAY, bits=1, channels=1)),
            ("dpi inch", make_tiff(rgb, 8, 2, tags={282: (5, [(300, 1)]), 283: (5, [(600, 2)]), 296: (3, [2])}),
             dict(dpix=300 << 16, dpiy=300 << 16, resunit=C.IMR_INCH)),
            ("dpi default unit", make_tiff(rgb, 8, 2, tags={282: (5, [(96, 1)]), 283: (5, [(96, 1)])}),
             dict(dpix=96 << 16, dpiy=96 << 16, resunit=C.IMR_INCH)),
            ("dpi cm", make_tiff(rgb, 8, 2, tags={282: (5, [(100, 1)]), 283: (5, [(50, 1)]), 296: (3, [3])}),
             dict(dpix=254 << 16, dpiy=127 << 16, resunit=C.IMR_CM)),
            ("dpi unit none", make_tiff(rgb, 8, 2, tags={282: (5, [(1, 1)]), 283: (5, [(2, 1)]), 296: (3, [1])}),
             dict(dpix=0, dpiy=0, resunit=C.IMR_NONE)),
            ("dpi zero denominator", make_tiff(rgb, 8, 2, tags={282: (5, [(300, 0)]), 283: (5, [(0, 0)])}),
             dict(dpix=0, dpiy=0))):
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT), it.expect_decex(None, meta=m), name))
    # orientation: reported and applied by the dispatcher (IMGD_NO_ORIENT keeps it)
    for o in range(0, 10):
        data = make_tiff(rgb, 8, 2, tags={274: (3, [o])})
        oo = o if 1 <= o <= 8 else 1
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT),
                        it.expect_decex(it.orient(exp, oo), meta={"orient": oo, "w": W, "h": H}), f"orientation {o}"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT | C.IMGD_NO_ORIENT),
                        it.expect_decex(exp, meta={"orient": oo}), f"orientation {o} not applied"))
    # blobs
    icc = it_icc_profile()
    xmp = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'>" + bytes(r.integers(65, 90, 300).astype(np.uint8)) + b"</x:xmpmeta>"
    data = make_tiff(rgb, 8, 2, tags={34675: (7, icc), 700: (1, xmp)})

    def bchk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        d = it.parse_decex(out)
        if d["icc"] != icc or d["xmp"] != xmp:
            return f"blobs icc {len(d['icc'])} xmp {len(d['xmp'])}"
        if d["meta"]["icclen"] != len(icc):
            return "icclen"
        return None
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT | C.IMGD_ICC | C.IMGD_XMP), bchk, "ICC + XMP"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT),
                    it.expect_decex(exp, meta={"icclen": 0, "xmplen": 0}), "blobs not requested"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_PROBE | C.IMGD_ICC),
                    lambda res, out: None if res == 0 and it.parse_decex(out)["icc"] == icc and
                    it.parse_decex(out)["w"] == 0 else "probe with ICC", "IMGD_PROBE + ICC"))
    # EXIF: image IFD with Exif and GPS sub-IFDs (II and MM, BigTIFF)
    for bo, big in (("<", False), (">", False), ("<", True)):
        pg = make_page(rgb, 8, 2, bo=bo)
        t = pg["tags"]
        t[0x010F] = (2, b"jfgui\0")
        t[0x0110] = (2, b"TIFF check\0")
        t[0x0112] = (3, [6])
        t[0x011A] = (5, [(72, 1)])
        t[0x0131] = (2, b"tiff_check.py\0")
        t[34665] = (13 if not big else 18, [0])               # patched below
        t[34853] = (4, [0])
        exif_ifd = {"tags": {0x829A: (5, [(1, 250)]), 0x9003: (2, b"2026:09:30 12:00:00\0"), 0xA001: (3, [1]),
                             0xA005: (4, [0]), 0x927C: (7, bytes(range(40)))}}
        gps_ifd = {"tags": {0x0000: (1, bytes([2, 3, 0, 0])), 0x0001: (2, b"N\0"),
                            0x0002: (5, [(48, 1), (8, 1), (0, 1)])}}
        # the sub-IFDs are extra IFDs outside the page chain; pointers from their positions
        _, pos = write_tiff([pg, exif_ifd, gps_ifd], bo=bo, big=big, nexts=[None, None, None], want_pos=True)
        t[34665] = (13 if not big else 18, [pos[1]])
        t[34853] = (4, [pos[2]])
        d = write_tiff([pg, exif_ifd, gps_ifd], bo=bo, big=big, nexts=[None, None, None])

        def echk(res, out, bo=bo):
            if res != 0:
                return f"error {it.err_name(res)}"
            dd = it.parse_decex(out)
            ex = dd["exif"]
            if not ex:
                return "no EXIF block"
            if ex[:2] != (b"II" if bo == "<" else b"MM"):
                return f"EXIF byte order {ex[:2]}"
            ifd0 = read_ifd_raw(ex, bo, struct.unpack_from(bo + "I", ex, 4)[0])
            if ifd0.get(0x0112, (0, [0]))[1][0] != 6:
                return "orientation not in IFD0"
            if 0x010F not in ifd0 or ifd0[0x010F][1][:6] != b"jfgui":
                return "Make missing"
            ep = ifd0.get(34665, (0, [0]))[1][0]
            gp = ifd0.get(34853, (0, [0]))[1][0]
            if not ep or not gp:
                return "sub-IFD pointers"
            e = read_ifd_raw(ex, bo, ep)
            g = read_ifd_raw(ex, bo, gp)
            if e.get(0x9003, (0, b""))[1] != b"2026:09:30 12:00:00\0" or 0xA005 in e:
                return "Exif IFD content"
            if e.get(0x927C, (0, b""))[1] != bytes(range(40)):
                return "MakerNote bytes"
            if g.get(0x0001, (0, b""))[1] != b"N\0" or g.get(0x0002, (0, []))[1] != [(48, 1), (8, 1), (0, 1)]:
                return "GPS IFD content"
            return None
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT | C.IMGD_EXIF), echk,
                        f"EXIF block {'MM' if bo == '>' else 'II'}{' BigTIFF' if big else ''}"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT), it.expect_decex(it.orient(exp, 6)),
                        f"EXIF file decodes {'MM' if bo == '>' else 'II'}"))
        exif_files = d
    # the EXIF block through exif_read (op 247): orientation 6, 72 dpi, ColorSpace 1
    rr = runner.one(it.OP_DECEX, 0, it.decex_input(exif_files, flags=C.IMGD_EXIF | C.IMGD_PROBE))
    if rr[0] == 0:
        ex = it.parse_decex(rr[1])["exif"]
        er = runner.one(it.OP_EXIFREAD, 0, ex)
        o, unit, dx, dy, cs, _ = struct.unpack_from("<6I", er[1], 0)
        suite.check(er[0] == 0 and o == 6 and cs == 1 and dx == 72 << 16, f"exif_read of the block: {er[0]} {o} {cs} {dx}")
    else:
        suite.fail("EXIF probe failed")
    # pages
    samples = [r.integers(0, 256, (H + k, W + 2 * k, 3)) for k in range(4)]
    pages = [make_page(s, 8, 2, comp=[1, 5, 8, 32773][k]) for k, s in enumerate(samples)]
    data = write_tiff(pages)
    for k in range(4):
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT, frame=k),
                        it.expect_decex(expected(samples[k], 8, 2), meta={"frames": 4}), f"page {k} of 4"))
    recs.append(err_rec(data, "page 4 of 4", C.ERR_LIMIT, frame=4))
    recs.append(err_rec(data, "page 1000 of 4", C.ERR_LIMIT, frame=1000))
    # BigTIFF / MM pages
    for bo, big in ((">", False), ("<", True), (">", True)):
        d = write_tiff([make_page(s, 8, 2, bo=bo) for s in samples[:3]], bo=bo, big=big)
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT, frame=2),
                        it.expect_decex(expected(samples[2], 8, 2), meta={"frames": 3}), f"pages {bo} big={big}"))
    # IFD chain loops: page count stops at the cycle
    d = write_tiff(pages[:3], nexts=[1, 2, 0])
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT, frame=2),
                    it.expect_decex(expected(samples[2], 8, 2), meta={"frames": 3}), "IFD chain loop (3 -> 0)"))
    d = write_tiff(pages[:2], nexts=[1, 1])
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT, frame=1),
                    it.expect_decex(expected(samples[1], 8, 2), meta={"frames": 2}), "IFD self loop"))
    # img_probe (op 241) and IMGD_PROBE
    d = write_tiff(pages)

    def pchk(res, out):
        if res != C.IMGF_TIFF:
            return f"probe {res}"
        m = it.parse_meta_rec(out)
        if (m["w"], m["h"], m["frames"], m["bits"], m["model"]) != (W, H, 4, 8, C.IMM_RGB):
            return f"meta {m}"
        return None
    recs.append(Rec(it.OP_PROBE, 0, d, pchk, "img_probe"))

    # format table: IC_CAPS / IC_IOM of tiff_codec through img_format_info (op 242)
    def fchk(res, out):
        caps, iom, reg = struct.unpack_from("<III", out, len(out) - 12)
        want_iom = C.IOM_BITDEPTH | C.IOM_DITHER | C.IOM_TIFFCOMP | C.IOM_ICC | C.IOM_XMP
        if not reg or (caps & (C.IFC_DECODE | C.IFC_ENCODE)) != C.IFC_DECODE | C.IFC_ENCODE:
            return f"caps {caps:#x} registered {reg}"
        if iom != want_iom:
            return f"iom {iom:#x} != {want_iom:#x}"
        return None
    recs.append(Rec(it.OP_FMTINFO, C.IMGF_TIFF, b"", fchk, "img_format_info(TIFF)"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=C.IMGD_PROBE, frame=3),
                    lambda res, out: None if res == 0 and it.parse_decex(out)["meta"]["w"] == W + 6 and
                    it.parse_decex(out)["w"] == 0 else f"probe page 3: {res}", "IMGD_PROBE page 3"))
    it.run_suite(runner, suite, recs)


def read_ifd_raw(ex, bo, off):
    n = struct.unpack_from(bo + "H", ex, off)[0]
    out = {}
    for i in range(n):
        tag, typ, cnt = struct.unpack_from(bo + "HHI", ex, off + 2 + 12 * i)
        sz = TSZ.get(typ, 1) * cnt
        voff = off + 2 + 12 * i + 8 if sz <= 4 else struct.unpack_from(bo + "I", ex, off + 2 + 12 * i + 8)[0]
        raw = ex[voff:voff + sz]
        if typ in (3, 4):
            out[tag] = (typ, list(struct.unpack_from(bo + TFMT[typ] * cnt, raw)))
        elif typ == 5:
            out[tag] = (typ, [struct.unpack_from(bo + "II", raw, 8 * k) for k in range(cnt)])
        else:
            out[tag] = (typ, bytes(raw))
    return out


# ================================================================== suite: errors

def run_errors(runner, suite, r):
    W, H = 19, 13
    rgb = r.integers(0, 256, (H, W, 3))
    good = make_tiff(rgb, 8, 2, comp=5, rps=4)
    recs = []
    E = C
    for comp in (6, 34712, 34925, 50000, 50001, 34887, 34676, 34677, 32771, 1234, 65535):
        recs.append(err_rec(make_tiff(rgb, 8, 2, tags={259: (3, [comp])}), f"compression {comp}", E.ERR_UNSUPPORTED))
    for ph in (4, 7, 9, 10, 32844, 32845, 65535):
        recs.append(err_rec(make_tiff(rgb, 8, 2, tags={262: (3, [ph])}), f"photometric {ph}", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={258: (3, [0, 0, 0])}), "BitsPerSample 0", E.ERR_HDR))
    for b in (33, 64, 65535):
        recs.append(err_rec(make_tiff(rgb, 8, 2, tags={258: (3, [b] * 3)}), f"BitsPerSample {b}", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={258: (3, [8, 16, 8])}), "mixed BitsPerSample", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={339: (3, [3] * 3)}), "float 8 bit", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={339: (3, [5] * 3)}), "SampleFormat 5", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={277: (3, [0])}), "SamplesPerPixel 0", E.ERR_HDR))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={277: (3, [17])}), "SamplesPerPixel 17", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb[..., :2], 8, 2), "RGB with 2 samples", E.ERR_HDR))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={284: (3, [3])}), "PlanarConfiguration 3", E.ERR_HDR))
    recs.append(err_rec(make_tiff(rgb, 8, 2, comp=5, tags={317: (3, [3])}), "predictor 3 on integers",
                        E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, comp=5, tags={317: (3, [4])}), "predictor 4", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 5, tags={332: (3, [2])}), "CMYK InkSet 2", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 16, 6, tags={530: (3, [2, 2])}), "YCbCr 16 bit", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 6, tags={530: (3, [3, 2])}), "YCbCr subsampling 3", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 3), "palette without ColorMap", E.ERR_HDR))
    recs.append(err_rec(make_tiff(rgb[..., 0], 32, 3, cmap=[0] * 3), "palette 32 bit", E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={256: (4, [0])}), "width 0", E.ERR_DIMS))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={257: (4, [70000])}), "height 70000", E.ERR_DIMS, E.ERR_HDR))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={256: None}), "width missing", E.ERR_DIMS))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tags={256: (4, [20000]), 257: (4, [20000])}), "400 MP", E.ERR_LIMIT))
    recs.append(err_rec(good, "maxpx 100", E.ERR_LIMIT, maxpx=100))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tile=(16, 16), tags={322: (3, [0])}), "TileWidth 0 (strips missing)",
                        E.ERR_HDR))
    recs.append(err_rec(make_tiff(rgb, 8, 2, tile=(16, 16), tags={322: (4, [70000])}), "TileWidth 70000",
                        E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(rgb, 8, 2, rps=4, tags={278: (4, [2])}), "too few strip offsets", E.ERR_HDR))
    pg = make_page(rgb, 8, 2, rps=4)
    pg["override"] = {273: None}
    recs.append(err_rec(write_tiff([pg]), "no StripOffsets", E.ERR_HDR))
    pg = make_page(rgb, 8, 2, rps=4)
    pg["override"] = {273: (4, [10 ** 9, 8, 8, 8])}
    recs.append(err_rec(write_tiff([pg]), "strip offset past the end", E.ERR_TRUNC))
    pg = make_page(rgb, 8, 2, rps=4)
    pg["override"] = {279: (4, [10 ** 9] * 4)}
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(write_tiff([pg]), flags=STRAIGHT),
                    it.expect_decex(expected(rgb, 8, 2)), "huge byte counts (uncompressed, clamped)"))
    pg = make_page(rgb, 8, 2, comp=5, rps=4)
    pg["override"] = {279: (4, [3] * 4)}
    recs.append(err_rec(write_tiff([pg]), "LZW strips cut short", E.ERR_TRUNC))
    for n in (0, 1, 2, 7, 8, 9, 20, len(good) // 2):
        recs.append(err_rec(good[:n], f"truncated at {n}", E.ERR_TRUNC, E.ERR_HDR, E.ERR_UNSUPPORTED))
    # cut inside the IFD values: the damaged entry is skipped (missing byte counts -> to the end)
    recs.append(err_rec(good[:-1], "last byte missing", 0, E.ERR_TRUNC, E.ERR_HDR))
    recs.append(err_rec(b"II*\0" + struct.pack("<I", 0), "IFD0 offset 0", E.ERR_HDR, E.ERR_TRUNC))
    recs.append(err_rec(b"II*\0" + struct.pack("<I", 8) + b"\xff\xff", "IFD with 65535 entries (clamped)",
                        E.ERR_DIMS, E.ERR_HDR, E.ERR_TRUNC))
    recs.append(err_rec(b"II+\0\x08\0\0\0" + struct.pack("<Q", 16) + struct.pack("<Q", 1 << 60), "BigTIFF huge count",
                        E.ERR_DIMS, E.ERR_HDR, E.ERR_TRUNC))
    recs.append(err_rec(b"MM\0*\0\0\0\x08\0\x01\x01\x00\0\x04\xff\xff\xff\xff\0\0\0\x10", "count beyond data",
                        E.ERR_DIMS, E.ERR_HDR, E.ERR_TRUNC))
    # bad compressed data
    pg = make_page(rgb, 8, 2, comp=8)
    pg["chunks"] = [b"\x78\x9c" + b"\xff" * 50]
    recs.append(err_rec(write_tiff([pg]), "bad deflate data", E.ERR_BLOCK, E.ERR_CODE, E.ERR_DIST, E.ERR_TRUNC,
                        E.ERR_HDR))
    pg["chunks"] = [b"\x78\x9d" + b"\0" * 10]
    recs.append(err_rec(write_tiff([pg]), "bad zlib header", E.ERR_HDR))
    pg = make_page(rgb, 8, 2, comp=5)
    pg["chunks"] = [bytes([0x80, 0x7F, 0xFF, 0xFF, 0xFF] + [0xAA] * 20)]
    recs.append(err_rec(write_tiff([pg]), "bad LZW codes", E.ERR_CODE, E.ERR_TRUNC))
    cc = make_tiff(np.zeros((8, 8), np.int64), 1, 0, comp=1, tags={259: (3, [4])})
    recs.append(err_rec(cc, "T.6 garbage", E.ERR_CODE, E.ERR_TRUNC, 0))
    jp = make_tiff(rgb, 8, 6, tags={259: (3, [7]), 530: (3, [1, 1])})
    recs.append(err_rec(jp, "JPEG garbage", E.ERR_HDR, E.ERR_TRUNC, E.ERR_UNSUPPORTED))
    recs.append(err_rec(make_tiff(r.integers(0, 256, (H, W, 4)), 8, 2, extra=[2], tags={259: (3, [7])}),
                        "JPEG with alpha", E.ERR_UNSUPPORTED))
    # cancel
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(good, flags=STRAIGHT, cancel=1), it.expect_err(E.ERR_CANCEL),
                    "cancel flag"))
    it.run_suite(runner, suite, recs)


# ================================================================== suite: fuzz

def fuzz_bases(r):
    W, H = 21, 15
    rgb = r.integers(0, 256, (H, W, 3))
    b = []
    for comp in (1, 5, -5, 8, 32773):
        b.append(make_tiff(rgb, 8, 2, comp=comp, rps=4))
        b.append(make_tiff(rgb, 16, 2, comp=comp, pred=2 if comp in (5, 8) else 1, tile=(16, 16), bo=">"))
    b.append(make_tiff(r.uniform(0, 1, (H, W, 3)).astype(np.float32), 32, 2, sfmt=3, comp=8, pred=3, big=True))
    b.append(make_tiff(r.integers(0, 16, (H, W)), 4, 3, cmap=r.integers(0, 65536, 48), comp=5))
    b.append(make_tiff(r.integers(0, 256, (H, W, 4)), 8, 5, planar=2, comp=32773))
    b.append(make_tiff(r.integers(0, 256, (H, W, 3)), 8, 8))
    b.append(make_tiff(r.integers(0, 2, (H, W)), 1, 0, comp=1, fill=2))
    b.append(ycc_sub_tiff(r.integers(0, 256, (H, W)), r.integers(0, 256, (8, 11)), r.integers(0, 256, (8, 11)), (2, 2),
                          comp=5))
    b.append(make_tiff(r.integers(0, 4096, (H, W, 2)), 12, 1, extra=[1], comp=5, pred=2))
    return b


def structural_mutants(r, base):
    """IFD-level corruption: huge / zero counts, offsets pointing into the IFD itself, next
    pointers to itself, wrong types."""
    out = []
    bo = "<" if base[:2] == b"II" else ">"
    big = base[2:4] in (b"+\0", b"\0+")
    if big:
        return out
    off = struct.unpack_from(bo + "I", base, 4)[0]
    n = struct.unpack_from(bo + "H", base, off)[0]
    for _ in range(12):
        b = bytearray(base)
        i = int(r.integers(0, n))
        e = off + 2 + 12 * i
        k = int(r.integers(0, 6))
        if k == 0:
            struct.pack_into(bo + "I", b, e + 4, int(r.choice([0, 1, 0xFFFF, 0x7FFFFFFF, 0xFFFFFFFF])))
        elif k == 1:
            struct.pack_into(bo + "I", b, e + 8, int(r.choice([0, off, off + 2, len(b) - 1, 0xFFFFFFF0])))
        elif k == 2:
            struct.pack_into(bo + "H", b, e + 2, int(r.integers(0, 20)))
        elif k == 3:
            struct.pack_into(bo + "I", b, off + 2 + 12 * n, off)       # next -> itself
        elif k == 4:
            struct.pack_into(bo + "H", b, off, int(r.choice([0, n + 1, 0xFFFF])))
        else:
            struct.pack_into(bo + "I", b, e + 8, int(r.integers(0, 1 << 32)))
        out.append(bytes(b))
    return out


def exif_file(r, bo="<", big=False):
    """A small RGB file with an Exif IFD and a GPS IFD (outside the page chain)."""
    rgb = r.integers(0, 256, (9, 11, 3))
    pg = make_page(rgb, 8, 2, bo=bo)
    t = pg["tags"]
    t[0x010F] = (2, b"jfgui\0")
    t[0x0112] = (3, [6])
    t[34665] = (13 if not big else 18, [0])
    t[34853] = (4, [0])
    ex = {"tags": {0x829A: (5, [(1, 250)]), 0x9003: (2, b"2026:09:30 12:00:00\0"), 0xA005: (4, [0]),
                   0x927C: (7, bytes(range(40)))}}
    gp = {"tags": {0x0000: (1, bytes([2, 3, 0, 0])), 0x0002: (5, [(48, 1), (8, 1), (0, 1)])}}
    _, pos = write_tiff([pg, ex, gp], bo=bo, big=big, nexts=[None, None, None], want_pos=True)
    t[34665] = (13 if not big else 18, [pos[1]])
    t[34853] = (4, [pos[2]])
    return write_tiff([pg, ex, gp], bo=bo, big=big, nexts=[None, None, None])


def lt_fuzz_bases(r, work):
    """Small CCITT and JPEG files written by libtiff (WSL)."""
    jobs, paths = [], []
    bl = (r.random((37, 61)) < 0.3).astype(np.uint8)
    rows = np.packbits(bl, axis=1)
    rb = rows.shape[1]
    raw = b"".join(rows[y:y + 8].tobytes().ljust(rb * 8, b"\0") for y in range(0, 37, 8))
    for comp, extra in ((2, []), (3, [(292, 0)]), (3, [(292, 5)]), (4, [])):
        p = Path(work) / f"fz_ccitt_{len(paths)}.tif"
        jobs.append({"op": "write", "path": wsl_path(p), "pages": [lt_page(
            [(256, 61), (257, 37), (258, 1), (277, 1), (259, comp), (262, 0), (278, 8), (284, 1)] + extra,
            raw, rb * 8)]})
        paths.append(p)
    img = r.integers(0, 256, (32, 48, 3)).astype(np.uint8)
    p = Path(work) / "fz_jpeg.tif"
    jobs.append({"op": "write", "path": wsl_path(p), "pages": [lt_page(
        [(256, 48), (257, 32), (258, 8), (277, 3), (259, 7), (262, 6), (65538, 1), (530, [2, 2]), (278, 16),
         (284, 1)], img.tobytes(), 48 * 3 * 16)]})
    paths.append(p)
    libtiff_run(jobs)
    return [pp.read_bytes() for pp in paths if pp.exists()]


def run_fuzz(runner, suite, r, n_cases, seed, lt_ok=False, work=None):
    rnd = random.Random(seed)
    bases = fuzz_bases(r)
    bases += [exif_file(r), exif_file(r, ">"), exif_file(r, "<", True)]
    bases.append(write_tiff([make_page(r.integers(0, 256, (7 + k, 9, 3)), 8, 2, comp=[1, 5, 8][k % 3])
                             for k in range(3)]))
    bases.append(make_tiff(r.integers(0, 256, (21, 35, 3)), 8, 2, comp=5, tile=(16, 16), planar=2, big=True))
    if lt_ok and work is not None:
        bases += lt_fuzz_bases(r, work)
    recs = []
    flags = [STRAIGHT, 0, STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP, C.IMGD_PROBE]
    for i in range(n_cases):
        base = bases[i % len(bases)]
        d = it.mutate(rnd, base, keep=0 if rnd.random() < 0.1 else 8)
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=flags[i % 4], frame=rnd.choice([0, 0, 0, 1])), None,
                        f"fuzz {i}"))
    for base in bases:
        for d in structural_mutants(r, base):
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT | C.IMGD_EXIF), None, "structural"))
    for i in range(0, len(recs), 2000):
        it.run_suite(runner, suite, recs[i:i + 2000])
    suite.notes.append(f"{len(recs)} mutated inputs")


# ================================================================== suite: par

def run_par(runner, suite, r, work, lt_ok, out, target):
    """Images of >= 1 MP with several strips / tiles go through the parallel decoder
    (thr_parallel_for): exact results, identical to --threads 1, deterministic errors."""
    W, H = 1031, 1025
    cases = []
    rgb = rand_img(r, W, H)
    cases.append(("RGB Deflate strips", make_tiff(rgb, 8, 2, comp=8, rps=16), expected(rgb, 8, 2)))
    cases.append(("RGB PackBits tiles 256 MM", make_tiff(rgb, 8, 2, comp=32773, tile=(256, 256), bo=">"),
                  expected(rgb, 8, 2)))
    v16 = r.integers(0, 65536, (H, W, 4))
    cases.append(("RGBA16 Deflate pred 2 tiles 128x64", make_tiff(v16, 16, 2, comp=8, pred=2, tile=(128, 64), extra=[2]),
                  expected(v16, 16, 2, alpha=3)))
    cm = r.integers(0, 256, (H, W, 4))
    cases.append(("CMYK planar PackBits strips", make_tiff(cm, 8, 5, comp=32773, planar=2, rps=40), expected(cm, 8, 5)))
    fg = r.uniform(0, 1, (H, W)).astype(np.float32)
    cases.append(("grey float32 pred 3 tiles", make_tiff(fg, 32, 1, sfmt=3, comp=8, pred=3, tile=(128, 64)),
                  expected(fg, 32, 1, sfmt=3)))
    Y = r.integers(0, 256, (H, W))
    Cb, Cr = r.integers(0, 256, (-(-H // 2), -(-W // 2))), r.integers(0, 256, (-(-H // 2), -(-W // 2)))
    cases.append(("YCbCr 2x2 Deflate strips", ycc_sub_tiff(Y, Cb, Cr, (2, 2), comp=8, rps=32),
                  ysub_expected(Y, Cb, Cr, (2, 2))))
    g4 = r.integers(0, 4, (H, W))
    cases.append(("grey 2-bit none strips", make_tiff(g4, 2, 0, rps=8), expected(g4, 2, 0)))
    # LZW strips from our encoder (the Python LZW is too slow for megapixels)
    px = np.concatenate([rgb[..., ::-1], np.full((H, W, 1), 255, np.uint8)], -1).astype(np.uint8)
    rr = runner.one(it.OP_ENCODE, 0, imgopt(bitdepth=24, comp=C.TIFC_LZW) + it.surface_input(px, C.FMT_ARGB))
    if rr[0] == 0:
        cases.append(("RGB LZW pred 2 strips (our encoder)", bytes(rr[1]), px))
    else:
        suite.fail(f"encode for par: {it.err_name(rr[0])}")
    # errors in two middle strips: the lower one is reported
    pg = make_page(rgb, 8, 2, comp=8, rps=16)
    pg["chunks"][20] = b"\x78\x9d" + pg["chunks"][20][2:]        # bad zlib header -> ERR_HDR
    pg["chunks"][40] = pg["chunks"][40][:10]                       # cut -> ERR_TRUNC / inflate error
    bad = write_tiff([pg])
    t1 = TRunner(target, out, timeout=600, work_name="tiffcheck1", extra=["--threads", "1"])
    for rn, tag in ((runner, ""), (t1, " (--threads 1)")):
        recs = [dec_rec(d, n + tag, e) for (n, d, e) in cases]
        recs.append(err_rec(bad, "errors in strips 20 and 40" + tag, C.ERR_HDR))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(cases[0][1], flags=STRAIGHT, cancel=1),
                        it.expect_err(C.ERR_CANCEL), "cancel" + tag))
        it.run_suite(rn, suite, recs)
    # CCITT / JPEG with many strips / tiles (libtiff references)
    if lt_ok and target == "win":
        jobs, names = [], []
        bl = (r.random((900, 1300)) < 0.02).astype(np.uint8)
        bl = np.maximum.accumulate(bl, axis=1) ^ np.roll(np.maximum.accumulate(bl, axis=1), 7, axis=1)
        rows = np.packbits(bl, axis=1)
        rb = rows.shape[1]
        raw = b"".join(rows[y:y + 64].tobytes().ljust(rb * 64, b"\0") for y in range(0, 900, 64))
        for comp, extra in ((4, []), (3, [[292, 1]])):
            p = Path(work) / f"par_ccitt_{comp}.tif"
            jobs.append({"op": "write", "path": wsl_path(p), "pages": [lt_page(
                [(256, 1300), (257, 900), (258, 1), (277, 1), (259, comp), (262, 0), (278, 64), (284, 1)] +
                [tuple(e) for e in extra], raw, rb * 64)]})
            names.append((f"CCITT {comp} 1300x900 strips", p))
        img = np.cumsum(r.integers(0, 6, (1024, 1024, 3)), axis=1).clip(0, 255).astype(np.uint8)
        tiles = b"".join(img[ty * 256:(ty + 1) * 256, tx * 256:(tx + 1) * 256].tobytes()
                         for ty in range(4) for tx in range(4))
        p = Path(work) / "par_jpeg.tif"
        jobs.append({"op": "write", "path": wsl_path(p), "pages": [lt_page(
            [(256, 1024), (257, 1024), (258, 8), (277, 3), (259, 7), (262, 6), (65538, 1), (530, [2, 2]),
             (322, 256), (323, 256), (284, 1)], tiles, 256 * 256 * 3, True)]})
        names.append(("JPEG tiles 1024x1024", p))
        libtiff_run(jobs)
        refs = libtiff_read([(pp, 0) for (_, pp) in names])
        for rn, tag in ((runner, ""), (t1, " (--threads 1)")):
            recs = []
            for (n, pp), ref in zip(names, refs):
                if ref is None or not pp.exists():
                    suite.fail(f"{n}: no libtiff reference")
                    continue
                recs.append(dec_rec(pp.read_bytes(), n + tag, ref))
            it.run_suite(rn, suite, recs)


# ================================================================== suite: perf

def run_perf(runner, suite, r, work):
    W, H = 4000, 3000
    yy, xx = np.mgrid[0:H, 0:W]
    img = np.stack([(xx * 255 // W), (yy * 255 // H), ((xx ^ yy) & 255)], -1).astype(np.uint8)
    img = (img + r.integers(0, 4, (H, W, 3))).astype(np.uint8)
    res = {}
    for name, comp, pred, one in (("LZW", 5, 1, 0), ("LZW + predictor", 5, 2, 0), ("Deflate + predictor", 8, 2, 0),
                                  ("Deflate", 8, 1, 0), ("none", 1, 1, 0), ("Deflate, one strip", 8, 1, 1)):
        rps = H if one else max(1, 65536 // (W * 3))
        if comp == 5:
            # the Python LZW is slow: encode with our encoder when possible (LZW + predictor 2)
            continue
        data = make_tiff(img, 8, 2, comp=comp, pred=pred, rps=rps)
        rr = runner.one(OP_TDEC, 3, data)
        if rr[0] != 0:
            suite.fail(f"perf {name}: {it.err_name(rr[0])}")
            continue
        us = struct.unpack_from("<I", rr[1], 0)[0]
        res[name] = us
    # LZW files from our encoder (predictor 2) and a plain LZW file from ffmpeg when available
    px = np.concatenate([img[..., ::-1], np.full((H, W, 1), 255, np.uint8)], -1)
    rr = runner.one(it.OP_ENCODE, 0, imgopt(bitdepth=24, comp=C.TIFC_LZW) + it.surface_input(px, C.FMT_ARGB))
    if rr[0] == 0:
        t = runner.one(OP_TDEC, 3, bytes(rr[1]))
        if t[0] == 0:
            res["LZW + predictor"] = struct.unpack_from("<I", t[1], 0)[0]
        te = runner.one(OP_TENC, 1, imgopt(bitdepth=24, comp=C.TIFC_LZW) + it.surface_input(px, C.FMT_ARGB))
        if te[0] == 0:
            suite.notes.append(f"encode 12 MP LZW + predictor: {struct.unpack_from('<I', te[1], 0)[0] / 1000:.0f} ms")
    if it.have_exe("ffmpeg"):
        p = Path(work) / "perf_lzw.tif"
        if it.ffmpeg_encode(px, p, ["-pix_fmt", "rgb24", "-compression_algo", "lzw"]):
            t = runner.one(OP_TDEC, 3, p.read_bytes())
            if t[0] == 0:
                res["LZW (ffmpeg)"] = struct.unpack_from("<I", t[1], 0)[0]
    for k, us in res.items():
        suite.notes.append(f"decode 12 MP {k}: {us / 1000:.0f} ms")
        if "LZW" in k or "Deflate" in k:
            suite.check(us < 250000, f"decode 12 MP {k}: {us / 1000:.0f} ms >= 250 ms")


# ================================================================== main

def main():
    ap = it.argparser(__doc__, SUITES)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(a.only.split(",")) if a.only else set(SUITES.split(","))
    r = np.random.default_rng(a.seed)
    suites = []
    ext = not a.no_ext
    lt_ok = ext and have_wsl_libtiff()
    for t in targets:
        runner = TRunner(t, out, timeout=600, work_name="tiffcheck")
        work = runner.work
        for name, fn in (("gen", lambda s: run_gen(runner, s, r, work, ext)),
                         ("wic", lambda s: run_wic(runner, s, r, work, lt_ok)),
                         ("ffmpeg", lambda s: run_ffmpeg(runner, s, r, work, lt_ok)),
                         ("libtiff", lambda s: run_libtiff(runner, s, r, work, lt_ok)),
                         ("encode", lambda s: run_encode(runner, s, r, work, ext, lt_ok)),
                         ("meta", lambda s: run_meta(runner, s, r, work)),
                         ("errors", lambda s: run_errors(runner, s, r)),
                         ("par", lambda s: run_par(runner, s, r, work, lt_ok, out, t)),
                         ("fuzz", lambda s: run_fuzz(runner, s, r, a.fuzz, a.seed, lt_ok, work)),
                         ("perf", lambda s: run_perf(runner, s, r, work))):
            if name not in only:
                continue
            if name in ("wic", "ffmpeg", "libtiff") and (not ext or t != "win"):
                continue
            s = it.Suite(name, t)
            r = np.random.default_rng(a.seed + len(suites))
            try:
                fn(s)
            except Exception as e:  # noqa: BLE001
                import traceback
                s.fail(f"suite exception {e!r}")
                s.msgs.append(traceback.format_exc()[-800:])
            suites.append(s)
            print(f"[{t}] {name}: {s.passed} passed, {s.failed} failed", flush=True)
    ok = it.report(suites, "tiff_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
