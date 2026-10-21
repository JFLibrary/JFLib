#!/usr/bin/env python3
"""JPEG codec checks for jfguiLib (img/jpeg_dec.s, jpeg_enc.s, jpeg_dsp.s).

Runs apps/codectest batch ops 20..24 (apps/codectest/src/jpeg_ops.s) on every
target and compares the results with references:

  pydec   files written by the JPEG writer in this script (baseline, SOF1,
          progressive with spectral selection + successive approximation and
          EOB runs, every sampling layout, restart intervals, grey / YCbCr /
          Adobe RGB / 'RGB' ids / CMYK / Adobe CMYK / YCCK, EXIF orientation in
          both byte orders, 16-bit DQT, DQT/DHT between scans, missing DHT,
          fill bytes, garbage, COM/APPn, missing EOI).  The expected image is
          computed from the known quantised coefficients with a libjpeg model
          (islow IDCT, fancy upsampling, jdcolor rounding) -- must match EXACTLY.
          ffmpeg decodes a subset of these files too (writer sanity check).
  ffdec   files encoded by ffmpeg (WSL; 4:4:4 / 4:2:2 / 4:2:0 / 4:4:0 / 4:1:1,
          q 2..31, default and optimal Huffman tables).  Reference 1: ffmpeg's
          own sample planes (-idct int, i.e. libjpeg islow) + the model's
          upsampling / colour conversion -> must match EXACTLY.  Reference 2:
          ffmpeg's RGB output (swscale, other upsampling) -> mean |d| < 1.5.
  gdidec  files encoded by Windows GDI+ (quality 5..100).  Reference 1 as
          above (exact); reference 2: GDI+'s decode -> mean |d| < 1.0, 99th
          percentile <= 12 (GDI+ uses its own IDCT and upsampling).
  enc     jpeg_encode: every size / format / quality; the file structure is
          checked (JFIF 96 dpi, DQT = libjpeg scaled tables, SOF0 sampling
          4:2:0 below q 90 and 4:4:4 from 90, stuffing, EOI), the files must be
          identical on all targets, decode with ffmpeg (bit-exact against our
          decoder via the model) and GDI+, and reach the PSNR thresholds
          (composited over white for PARGB).
  errors  error codes for broken / unsupported files.
  damaged truncated sequential files and corrupted entropy-coded data (bit flips,
          renumbered / deleted / inserted RST markers): whenever we return an
          image it must equal libjpeg's (zero fill, grey rest of the restart
          interval, restart resync); truncated progressive files -> a partial
          image of the right size.
  fuzz    truncations, bit flips, garbage segments, bad lengths -> an error
          code or a well-formed image, never a crash (inputs end at a guard page).
  bench   decode of a 12 MP 4:2:0 photo (target < 250 ms), encode of
          1152 x 648 (target < 25 ms).
  CA package (ops 25..27, 240, 252, 260, 261):
  flags   jpeg_decode_fl: JDF_NO_ORIENT / IMGD_NO_ORIENT keep the stored orientation (IM_ORIENT
          reported, IMF_ORIENT_APPLIED clear), JDF_STRAIGHT (FMT_ARGB), JDF_RGB / JDF_YCC /
          JDF_CMYK force the colour interpretation -- exact against the model
  abbrev  jpeg_decode_abbrev: files split into a tables-only stream + an abbreviated image
          (every layout, restart intervals, no DHT at all, image tables overriding) == model;
          error cases (SOF in the tables, no SOI, no DQT, empty image)
  encex   jpeg_encode_ex / img_encode: chroma 4:4:4 / 4:2:2 / 4:2:0 / 4:0:0 / default rule
          (SOF sampling), JFIF density, ARGB input == its premultiplied copy byte for byte,
          identical on all targets, our decoder == libjpeg-turbo, PSNR vs the source
  jmeta   img_encode with IO_META: APP1 EXIF (orientation 1), APP2 ICC (1 and 3 segments),
          APP1 XMP, density; oversized EXIF / XMP skipped; read back through op 261
  classic jpeg_encode / jpeg_decode byte-identical to the C0 reference binary
          (build/ca/ref/codectest_c0.exe, when present)
  jmfuzz  mutated JPEGs with metadata through op 240 (all blob flags) / 261 / 25 and
          mutated tables / image pairs through op 26

  python tools/jpeg_check.py --target both --out build/agent-jpeg
  python tools/jpeg_check.py --target win --only pydec,enc --no-build
"""

import argparse
import math
import random
import shlex
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
import testenv  # noqa: E402
from codec_check import ERR_NAMES, Rec, Runner, Suite, run_suite  # noqa: E402

OP_JDEC, OP_JENC, OP_JHASH, OP_JBDEC, OP_JBENC = 20, 21, 22, 23, 24
WSL = ["wsl.exe", "-d", build.WSL_DISTRO]

ZZ = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34,
      27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
      58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63]
STD_QL = [16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
          14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92,
          49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99]
STD_QC = [17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
          47, 66, 99, 99, 99, 99, 99, 99] + [99] * 32
STD_HUFF = {
    (0, 0): ([0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], list(range(12))),
    (0, 1): ([0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], list(range(12))),
    (1, 0): ([0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d], bytes.fromhex(
        "01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728"
        "292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a83848586878889"
        "8a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2"
        "e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9fa")),
    (1, 1): ([0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77], bytes.fromhex(
        "000102031104052131061241510761711322328108144291a1b1c109233352f0156272d10a162434e125f11718191a26"
        "2728292a35363738393a434445464748494a535455565758595a636465666768696a737475767778797a828384858687"
        "88898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9da"
        "e2e3e4e5e6e7e8e9eaf2f3f4f5f6f7f8f9fa")),
}


def rel(p):
    return str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")


# ================================================================ libjpeg model

def scaled_q(base, quality):
    q = max(1, min(100, quality))
    s = 5000 // q if q < 50 else 200 - 2 * q
    return [min(255, max(1, (b * s + 50) // 100)) for b in base]


def _idct1d(i, shift):
    i0, i1, i2, i3, i4, i5, i6, i7 = i
    z1 = (i2 + i6) * 4433
    tmp2 = z1 + i6 * -15137
    tmp3 = z1 + i2 * 6270
    tmp0 = (i0 + i4) << 13
    tmp1 = (i0 - i4) << 13
    tmp10, tmp13, tmp11, tmp12 = tmp0 + tmp3, tmp0 - tmp3, tmp1 + tmp2, tmp1 - tmp2
    t0, t1, t2, t3 = i7, i5, i3, i1
    z1, z2, z3, z4 = t0 + t3, t1 + t2, t0 + t2, t1 + t3
    z5 = (z3 + z4) * 9633
    t0, t1, t2, t3 = t0 * 2446, t1 * 16819, t2 * 25172, t3 * 12299
    z1, z2, z3, z4 = z1 * -7373, z2 * -20995, z3 * -16069 + z5, z4 * -3196 + z5
    t0 = t0 + z1 + z3
    t1 = t1 + z2 + z4
    t2 = t2 + z2 + z3
    t3 = t3 + z1 + z4
    r = 1 << (shift - 1)
    return [(tmp10 + t3 + r) >> shift, (tmp11 + t2 + r) >> shift, (tmp12 + t1 + r) >> shift,
            (tmp13 + t0 + r) >> shift, (tmp13 - t0 + r) >> shift, (tmp12 - t1 + r) >> shift,
            (tmp11 - t2 + r) >> shift, (tmp10 - t3 + r) >> shift]


def idct_islow(coefs, q):
    """coefs (..., 64) natural order, q (64,) -> (..., 8, 8) uint8, libjpeg jpeg_idct_islow."""
    c = (np.asarray(coefs, np.int64) * np.asarray(q, np.int64)).reshape(coefs.shape[:-1] + (8, 8))
    ws = _idct1d([c[..., k, :] for k in range(8)], 11)           # columns: k = row index
    ws = np.stack(ws, -2)
    out = _idct1d([ws[..., :, k] for k in range(8)], 18)         # rows
    out = np.stack(out, -1) + 128
    return np.clip(out, 0, 255).astype(np.uint8)


def planes_from_coefs(coefs, q):
    """coefs (BH, BW, 64) -> sample plane (BH*8, BW*8)."""
    bh, bw, _ = coefs.shape
    blk = idct_islow(coefs, q)                                   # (bh, bw, 8, 8)
    return blk.transpose(0, 2, 1, 3).reshape(bh * 8, bw * 8)


def _edge_prev(a, axis):
    first = np.take(a, [0], axis)
    return np.concatenate([first, np.take(a, range(a.shape[axis] - 1), axis)], axis)


def _edge_next(a, axis):
    last = np.take(a, [a.shape[axis] - 1], axis)
    return np.concatenate([np.take(a, range(1, a.shape[axis]), axis), last], axis)


def upsample(plane, dw, dh, h, v, hmax, vmax, W, H):
    """libjpeg(-turbo) upsampling of a component (dh x dw samples) to H x W."""
    s = plane[:dh, :dw].astype(np.int64)
    if h == hmax and v == vmax:
        out = s
    elif 2 * h == hmax and dw <= 2:                         # libjpeg: no fancy h2v1 / h2v2 for tiny widths
        out = s[(np.arange(H) * v) // vmax][:, (np.arange(W) * h) // hmax]
    elif 2 * h == hmax and v == vmax:                       # h2v1 fancy
        out = np.empty((dh, 2 * dw), np.int64)
        out[:, 0::2] = (3 * s + _edge_prev(s, 1) + 1) >> 2
        out[:, 1::2] = (3 * s + _edge_next(s, 1) + 2) >> 2
    elif 2 * h == hmax and 2 * v == vmax:                   # h2v2 fancy
        out = np.empty((2 * dh, 2 * dw), np.int64)
        for par, nbr in ((0, _edge_prev(s, 0)), (1, _edge_next(s, 0))):
            c = 3 * s + nbr
            out[par::2, 0::2] = (3 * c + _edge_prev(c, 1) + 8) >> 4
            out[par::2, 1::2] = (3 * c + _edge_next(c, 1) + 7) >> 4
    elif h == hmax and 2 * v == vmax:                       # h1v2 fancy (libjpeg-turbo)
        out = np.empty((2 * dh, dw), np.int64)
        out[0::2] = (3 * s + _edge_prev(s, 0) + 1) >> 2
        out[1::2] = (3 * s + _edge_next(s, 0) + 2) >> 2
    else:                                                   # replicate
        ys = (np.arange(H) * v) // vmax
        xs = (np.arange(W) * h) // hmax
        out = s[ys][:, xs]
    return out[:H, :W]


def ycc_rgb(y, cb, cr):
    y, x, z = (np.asarray(a, np.int64) for a in (y, cb, cr))
    x, z = x - 128, z - 128
    r = y + ((91881 * z + 32768) >> 16)
    g = y + ((-22554 * x - 46802 * z + 32768) >> 16)
    b = y + ((116130 * x + 32768) >> 16)
    return np.clip(np.stack([r, g, b], -1), 0, 255)


def div255(t):
    t = t + 128
    return (t + (t >> 8)) >> 8


def model_rgb(comp_rows, cspace, cmode=0):
    """full-resolution component planes -> (H, W, 3) RGB."""
    if cspace == "gray":
        g = comp_rows[0]
        return np.stack([g, g, g], -1)
    if cspace == "rgb":
        return np.stack(comp_rows[:3], -1)
    if cspace == "ycc":
        return ycc_rgb(*comp_rows[:3])
    c, m, y, k = (np.asarray(a, np.int64) for a in comp_rows)
    if cspace == "ycck":
        rgb = ycc_rgb(c, m, y)
        return div255((255 - rgb) * k[..., None])
    if cmode == 1:                                  # plain CMYK
        return div255((255 - np.stack([c, m, y], -1)) * (255 - k)[..., None])
    return div255(np.stack([c, m, y], -1) * k[..., None])   # Adobe (inverted)


def orient(img, o):
    if o == 2:
        return img[:, ::-1]
    if o == 3:
        return img[::-1, ::-1]
    if o == 4:
        return img[::-1]
    if o == 5:
        return img.transpose(1, 0, 2)
    if o == 6:
        return img[::-1].transpose(1, 0, 2)
    if o == 7:
        return img[::-1, ::-1].transpose(1, 0, 2)
    if o == 8:
        return img[:, ::-1].transpose(1, 0, 2)
    return img


def bgra_to_rgb(out, w, h):
    px = np.frombuffer(out, np.uint8, offset=8).reshape(h, w, 4)
    return px[..., 2::-1].astype(np.int64), px[..., 3]


# ================================================================ Python JPEG writer

def dct_matrix():
    c = np.zeros((8, 8))
    for u in range(8):
        for x in range(8):
            c[u, x] = (math.sqrt(0.5) if u == 0 else 1.0) * 0.5 * math.cos((2 * x + 1) * u * math.pi / 16)
    return c


DCT = dct_matrix()


def fdct_quant(plane, q):
    """plane (BH*8, BW*8) samples -> (BH, BW, 64) quantised coefficients (natural order)."""
    bh, bw = plane.shape[0] // 8, plane.shape[1] // 8
    b = plane.reshape(bh, 8, bw, 8).transpose(0, 2, 1, 3).astype(np.float64) - 128.0
    f = np.einsum("ux,abxy,vy->abuv", DCT, b, DCT)
    qq = np.asarray(q, np.float64).reshape(8, 8)
    r = np.sign(f) * np.floor(np.abs(f) / qq + 0.5)
    r = r.reshape(bh, bw, 64)
    r[..., 1:] = np.clip(r[..., 1:], -1023, 1023)
    r[..., 0] = np.clip(r[..., 0], -1024, 1023)
    return r.astype(np.int64)


def gen_optimal_table(freq_in):
    """libjpeg jpeg_gen_optimal_table -> (bits[16], vals)."""
    freq = list(freq_in) + [1]
    if sum(freq_in) == 0:
        return [1] + [0] * 15, [0]
    codesize, others = [0] * 257, [-1] * 257
    while True:
        c1, v = -1, 1 << 62
        for i in range(257):
            if freq[i] and freq[i] <= v:
                v, c1 = freq[i], i
        c2, v = -1, 1 << 62
        for i in range(257):
            if freq[i] and freq[i] <= v and i != c1:
                v, c2 = freq[i], i
        if c2 < 0:
            break
        freq[c1] += freq[c2]
        freq[c2] = 0
        codesize[c1] += 1
        while others[c1] >= 0:
            c1 = others[c1]
            codesize[c1] += 1
        others[c1] = c2
        codesize[c2] += 1
        while others[c2] >= 0:
            c2 = others[c2]
            codesize[c2] += 1
    bits = [0] * 300
    for i in range(257):
        if codesize[i]:
            bits[codesize[i]] += 1
    for i in range(299, 16, -1):
        while bits[i] > 0:
            j = i - 2
            while bits[j] == 0:
                j -= 1
            bits[i] -= 2
            bits[i - 1] += 1
            bits[j + 1] += 2
            bits[j] -= 1
    i = 16
    while bits[i] == 0:
        i -= 1
    bits[i] -= 1
    vals = [j for size in range(1, 300) for j in range(256) if codesize[j] == size]
    return bits[1:17], vals


def huff_codes(bits, vals):
    codes, code, k = {}, 0, 0
    for ln in range(1, 17):
        for _ in range(bits[ln - 1]):
            codes[vals[k]] = (code, ln)
            code += 1
            k += 1
        code <<= 1
    return codes


class BitWriter:
    def __init__(self):
        self.out = bytearray()
        self.acc = 0
        self.n = 0

    def put(self, v, n):
        if n <= 0:
            return
        self.acc = (self.acc << n) | (v & ((1 << n) - 1))
        self.n += n
        while self.n >= 8:
            self.n -= 8
            b = (self.acc >> self.n) & 0xFF
            self.out.append(b)
            if b == 0xFF:
                self.out.append(0)
        self.acc &= (1 << self.n) - 1

    def flush(self):
        if self.n:
            self.put((1 << (8 - self.n)) - 1, 8 - self.n)


def nbits(v):
    return int(abs(int(v))).bit_length()


class ScanEvents:
    """Symbol / raw-bit event list of one scan; tables are chosen afterwards."""

    def __init__(self):
        self.ev = []
        self.eobrun = 0
        self.be = []

    def sym(self, cls, tid, s):
        self.ev.append((0, cls, tid, s))

    def bits(self, v, n):
        if n:
            self.ev.append((1, v, n))

    def emit_eobrun(self, tid):
        if self.eobrun > 0:
            t, nb = self.eobrun, 0
            while t >> 1:
                t >>= 1
                nb += 1
            self.sym(1, tid, nb << 4)
            self.bits(self.eobrun, nb)
            self.eobrun = 0
            for b in self.be:
                self.bits(b, 1)
            self.be = []


def enc_seq_block(ev, blk, ci, pred, dct, act):
    diff = int(blk[0]) - pred[ci]
    pred[ci] = int(blk[0])
    s = nbits(diff)
    ev.sym(0, dct, s)
    ev.bits(diff if diff >= 0 else diff - 1, s)
    r = 0
    for k in range(1, 64):
        v = int(blk[ZZ[k]])
        if v == 0:
            r += 1
            continue
        while r > 15:
            ev.sym(1, act, 0xF0)
            r -= 16
        s = nbits(v)
        ev.sym(1, act, (r << 4) | s)
        ev.bits(v if v >= 0 else v - 1, s)
        r = 0
    if r:
        ev.sym(1, act, 0)


def enc_dc_first(ev, blk, ci, pred, dct, al):
    t = int(blk[0]) >> al
    diff = t - pred[ci]
    pred[ci] = t
    s = nbits(diff)
    ev.sym(0, dct, s)
    ev.bits(diff if diff >= 0 else diff - 1, s)


def enc_ac_first(ev, blk, act, ss, se, al):
    r = 0
    for k in range(ss, se + 1):
        v = int(blk[ZZ[k]])
        if v == 0:
            r += 1
            continue
        t = abs(v) >> al
        if t == 0:
            r += 1
            continue
        t2 = t if v > 0 else ~t
        ev.emit_eobrun(act)
        while r > 15:
            ev.sym(1, act, 0xF0)
            r -= 16
        s = nbits(t)
        ev.sym(1, act, (r << 4) | s)
        ev.bits(t2, s)
        r = 0
    if r > 0:
        ev.eobrun += 1
        if ev.eobrun == 0x7FFF:
            ev.emit_eobrun(act)


def enc_ac_refine(ev, blk, act, ss, se, al):
    absv = [0] * 64
    eob = 0
    for k in range(ss, se + 1):
        t = abs(int(blk[ZZ[k]])) >> al
        absv[k] = t
        if t == 1:
            eob = k
    r = 0
    br = []
    for k in range(ss, se + 1):
        t = absv[k]
        if t == 0:
            r += 1
            continue
        while r > 15 and k <= eob:
            ev.emit_eobrun(act)
            ev.sym(1, act, 0xF0)
            r -= 16
            for b in br:
                ev.bits(b, 1)
            br = []
        if t > 1:
            br.append(t & 1)
            continue
        ev.emit_eobrun(act)
        ev.sym(1, act, (r << 4) | 1)
        ev.bits(0 if int(blk[ZZ[k]]) < 0 else 1, 1)
        for b in br:
            ev.bits(b, 1)
        br = []
        r = 0
    if r > 0 or br:
        ev.eobrun += 1
        ev.be += br
        if ev.eobrun == 0x7FFF or len(ev.be) > 1000 - 64 + 1:
            ev.emit_eobrun(act)


def seg(marker, payload):
    return bytes([0xFF, marker]) + struct.pack(">H", len(payload) + 2) + payload


def dht_payload(tables):
    out = bytearray()
    for (cls, tid), (bits, vals) in tables:
        out.append((cls << 4) | tid)
        out += bytes(bits)
        out += bytes(vals)
    return bytes(out)


def exif_app1(o, big=False, extra=True):
    e = ">" if big else "<"
    entries = []
    data = b""
    base = 8
    n = 3 if extra else 1
    data_off = base + 2 + 12 * n + 4
    if extra:
        make = b"jfgui\x00"
        entries.append(struct.pack(e + "HHII", 0x010F, 2, len(make), data_off))
        data += make
        entries.append(struct.pack(e + "HHI", 0x0110, 2, 4) + b"cam\x00")
    entries.append(struct.pack(e + "HHIH", 0x0112, 3, 1, o) + b"\x00\x00")
    ifd = struct.pack(e + "H", len(entries)) + b"".join(entries) + struct.pack(e + "I", 0)
    tiff = (b"MM\x00\x2a" if big else b"II\x2a\x00") + struct.pack(e + "I", 8) + ifd + data
    return seg(0xE1, b"Exif\x00\x00" + tiff)


class JpegSpec:
    """Everything the writer needs; `coefs` / `q` also drive the reference decode."""

    def __init__(self, W, H, samp, ids=None):
        self.W, self.H, self.samp = W, H, samp
        self.nc = len(samp)
        self.ids = ids or list(range(1, self.nc + 1))
        self.hmax = max(h for h, v in samp)
        self.vmax = max(v for h, v in samp)
        self.mcux = -(-W // (8 * self.hmax))
        self.mcuy = -(-H // (8 * self.vmax))
        self.geo = []
        for h, v in samp:
            dw = -(-W * h // self.hmax)
            dh = -(-H * v // self.vmax)
            self.geo.append(dict(h=h, v=v, bw=self.mcux * h, bh=self.mcuy * v, dw=dw, dh=dh,
                                 nbw=-(-dw // 8), nbh=-(-dh // 8)))


def comp_planes_from(spec, full_planes):
    """full-resolution float planes (H x W) -> per component padded sample planes."""
    out = []
    for g, p in zip(spec.geo, full_planes):
        p = np.asarray(p, np.float64)
        fy, fx = spec.vmax / g["v"], spec.hmax / g["h"]
        if fy == int(fy) and fx == int(fx):
            fy, fx = int(fy), int(fx)
            ph, pw = g["dh"] * fy, g["dw"] * fx
            pp = np.pad(p, ((0, max(0, ph - p.shape[0])), (0, max(0, pw - p.shape[1]))), mode="edge")[:ph, :pw]
            d = pp.reshape(g["dh"], fy, g["dw"], fx).mean(axis=(1, 3))
        else:
            ys = np.minimum((np.arange(g["dh"]) * spec.vmax) // g["v"], spec.H - 1)
            xs = np.minimum((np.arange(g["dw"]) * spec.hmax) // g["h"], spec.W - 1)
            d = p[ys][:, xs]
        d = np.clip(np.floor(d + 0.5), 0, 255)
        d = np.pad(d, ((0, g["bh"] * 8 - g["dh"]), (0, g["bw"] * 8 - g["dw"])), mode="edge")
        out.append(d)
    return out


def rgb_to_ycc(rgb):
    r, g, b = (rgb[..., k].astype(np.float64) for k in range(3))
    y = 0.299 * r + 0.587 * g + 0.114 * b
    cb = -0.168736 * r - 0.331264 * g + 0.5 * b + 128
    cr = 0.5 * r - 0.418688 * g - 0.081312 * b + 128
    return [y, cb, cr]


def default_script(nc, sa=True):
    if nc == 3 and sa:
        return [([0, 1, 2], 0, 0, 0, 1), ([0], 1, 5, 0, 2), ([2], 1, 63, 0, 1), ([1], 1, 63, 0, 1),
                ([0], 6, 63, 0, 2), ([0], 1, 63, 2, 1), ([0, 1, 2], 0, 0, 1, 0), ([2], 1, 63, 1, 0),
                ([1], 1, 63, 1, 0), ([0], 1, 63, 1, 0)]
    comps = list(range(nc))
    if not sa:
        return [(comps, 0, 0, 0, 0)] + [([c], 1, 5, 0, 0) for c in comps] + [([c], 6, 63, 0, 0) for c in comps]
    s = [(comps, 0, 0, 0, 1)]
    s += [([c], 1, 5, 0, 2) for c in comps]
    s += [([c], 6, 63, 0, 2) for c in comps]
    s += [([c], 1, 63, 2, 1) for c in comps]
    s += [(comps, 0, 0, 1, 0)]
    s += [([c], 1, 63, 1, 0) for c in comps]
    return s


def write_jpeg(spec, coefs, qtabs, qsel, *, mode="baseline", scans=None, restart=0, huff="opt",
               tsel=None, jfif=True, adobe=None, exif=None, com=True, q16=False, late_dqt=False,
               fill=False, garbage=False, eoi=True, extra_app=False, trailing=b""):
    """coefs: per component (BH, BW, 64) quantised; qtabs {id: 64 natural}; qsel per component."""
    nc = spec.nc
    tsel = tsel or [(0 if c == 0 else 1, 0 if c == 0 else 1) for c in range(nc)]
    out = bytearray(b"\xFF\xD8")

    def put(b):
        if fill:
            out.extend(b"\xFF\xFF")
        if garbage:
            out.extend(b"\x12\x34")
        out.extend(b)
    if jfif:
        put(seg(0xE0, b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00"))
    if exif is not None:
        put(exif_app1(*exif) if isinstance(exif, tuple) else exif_app1(exif))
    if adobe is not None:
        put(seg(0xEE, b"Adobe" + struct.pack(">HHHB", 100, 0, 0, adobe)))
    if extra_app:
        put(seg(0xE2, b"ICC_PROFILE\x00" + bytes(40)))
        put(seg(0xED, b"Photoshop 3.0\x00" + bytes(20)))
    if com:
        put(seg(0xFE, b"written by jpeg_check.py"))

    def dqt(ids):
        p = bytearray()
        for i in ids:
            if q16:
                p.append(0x10 | i)
                p += b"".join(struct.pack(">H", qtabs[i][ZZ[k]]) for k in range(64))
            else:
                p.append(i)
                p += bytes(qtabs[i][ZZ[k]] for k in range(64))
        return seg(0xDB, bytes(p))
    early = sorted(qtabs) if not late_dqt else [qsel[0]]
    put(dqt(early))
    if restart:
        put(seg(0xDD, struct.pack(">H", restart)))
    sofm = {"baseline": 0xC0, "ext": 0xC1, "prog": 0xC2}[mode]
    p = struct.pack(">BHHB", 8, spec.H, spec.W, nc)
    for c in range(nc):
        h, v = spec.samp[c]
        p += bytes([spec.ids[c], (h << 4) | v, qsel[c]])
    put(seg(sofm, p))
    if scans is None:
        scans = default_script(nc) if mode == "prog" else [(list(range(nc)), 0, 63, 0, 0)]
    if huff == "std":
        tabs = [((cls, tid), STD_HUFF[(cls, min(tid, 1))]) for cls in (0, 1) for tid in sorted(set(t[cls] for t in tsel))]
        put(seg(0xC4, dht_payload(tabs)))
    dqt_done = set(early)
    for si, (sc, ss, se, ah, al) in enumerate(scans):
        need = [qsel[c] for c in sc if qsel[c] not in dqt_done]
        if need:
            put(dqt(sorted(set(need))))
            dqt_done |= set(need)
        ev = ScanEvents()
        pred = [0] * nc
        inter = len(sc) > 1
        if inter:
            mcus = [[(c, my * spec.samp[c][1] + by, mx * spec.samp[c][0] + bx)
                     for c in sc for by in range(spec.samp[c][1]) for bx in range(spec.samp[c][0])]
                    for my in range(spec.mcuy) for mx in range(spec.mcux)]
        else:
            g = spec.geo[sc[0]]
            mcus = [[(sc[0], by, bx)] for by in range(g["nbh"]) for bx in range(g["nbw"])]
        act0 = tsel[sc[0]][1]
        for m, mcu in enumerate(mcus):
            if restart and m and m % restart == 0:
                if mode == "prog":
                    ev.emit_eobrun(act0)
                ev.ev.append((2, (m // restart - 1) & 7))
                pred = [0] * nc
            for (c, by, bx) in mcu:
                blk = coefs[c][by, bx]
                dct, act = tsel[c]
                if mode != "prog":
                    enc_seq_block(ev, blk, c, pred, dct, act)
                elif ss == 0 and ah == 0:
                    enc_dc_first(ev, blk, c, pred, dct, al)
                elif ss == 0:
                    ev.bits((int(blk[0]) >> al) & 1, 1)
                elif ah == 0:
                    enc_ac_first(ev, blk, act, ss, se, al)
                else:
                    enc_ac_refine(ev, blk, act, ss, se, al)
        if mode == "prog":
            ev.emit_eobrun(act0)
        # tables of this scan
        if huff == "std":
            codes = {(cls, tid): huff_codes(*STD_HUFF[(cls, min(tid, 1))]) for cls in (0, 1) for tid in range(4)}
        else:
            freq = {}
            for e in ev.ev:
                if e[0] == 0:
                    freq.setdefault((e[1], e[2]), [0] * 256)[e[3]] += 1
            tabs = [(k, gen_optimal_table(freq[k])) for k in sorted(freq)]
            codes = {k: huff_codes(*t) for k, t in tabs}
            if huff == "none":          # MJPEG style: rely on the decoder's default tables
                codes = {(cls, tid): huff_codes(*STD_HUFF[(cls, min(tid, 1))]) for cls in (0, 1) for tid in range(4)}
            elif tabs:
                put(seg(0xC4, dht_payload(tabs)))
        p = bytes([len(sc)])
        for c in sc:
            p += bytes([spec.ids[c], (tsel[c][0] << 4) | tsel[c][1]])
        p += bytes([ss, se, (ah << 4) | al])
        put(seg(0xDA, p))
        bw = BitWriter()
        for e in ev.ev:
            if e[0] == 0:
                code, ln = codes[(e[1], e[2])][e[3]]
                bw.put(code, ln)
            elif e[0] == 1:
                bw.put(e[1], e[2])
            else:
                bw.flush()
                bw.out += bytes([0xFF, 0xD0 + e[1]])
        bw.flush()
        out += bw.out
    if eoi:
        if fill:
            out += b"\xFF\xFF"
        out += b"\xFF\xD9"
    out += trailing
    return bytes(out)


# ================================================================ test images

def photo(rng, w, h, seed_shift=0):
    """photo-like RGB: smooth gradients, soft shapes, texture, a few edges."""
    r = np.random.default_rng(rng.randrange(1 << 30) + seed_shift)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    img = np.zeros((h, w, 3))
    for k in range(3):
        fx, fy = r.uniform(0.5, 3) / max(w, 1), r.uniform(0.5, 3) / max(h, 1)
        img[..., k] = 128 + 90 * np.sin(2 * np.pi * (fx * xx + fy * yy) + r.uniform(0, 6))
    for _ in range(6):
        cx, cy, rad = r.uniform(0, w), r.uniform(0, h), r.uniform(0.05, 0.3) * max(w, h)
        col = r.uniform(0, 255, 3)
        m = np.clip(1.5 - np.hypot(xx - cx, yy - cy) / rad, 0, 1)[..., None]
        img = img * (1 - m) + col * m
    x0, y0 = int(r.integers(0, max(1, w))), int(r.integers(0, max(1, h)))
    img[y0:y0 + max(1, h // 5), x0:x0 + max(1, w // 4)] = r.uniform(0, 255, 3)
    img += r.normal(0, 4, img.shape)
    return np.clip(img + 0.5, 0, 255).astype(np.uint8)


def bmp24(rgb):
    h, w, _ = rgb.shape
    stride = (w * 3 + 3) & ~3
    rows = bytearray()
    for y in range(h - 1, -1, -1):
        row = rgb[y, :, ::-1].tobytes()
        rows += row + bytes(stride - len(row))
    hdr = struct.pack("<2sIHHI", b"BM", 54 + len(rows), 0, 0, 54)
    info = struct.pack("<IiiHHIIiiII", 40, w, h, 1, 24, 0, len(rows), 2835, 2835, 0, 0)
    return hdr + info + bytes(rows)


def ppm(rgb):
    h, w, _ = rgb.shape
    return b"P6 %d %d 255\n" % (w, h) + rgb.astype(np.uint8).tobytes()


# ================================================================ external tools

class Ext:
    """ffmpeg in WSL and GDI+ (PowerShell), batched."""

    def __init__(self, work):
        self.work = work
        self.work.mkdir(parents=True, exist_ok=True)
        self.n = 0

    def path(self, name):
        return self.work / name

    def ffmpeg(self, cmds):
        """cmds: list of argument lists (paths relative to the repo root)."""
        if not cmds:
            return
        self.n += 1
        sh = self.work / f"ff{self.n}.sh"
        lines = ["set -e"]
        for c in cmds:
            lines.append(" ".join(shlex.quote(a) for a in ["ffmpeg", "-nostdin", "-y", "-loglevel", "error"] + c))
        sh.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        p = subprocess.run(WSL + ["--cd", str(ROOT), "-e", "sh", rel(sh)], capture_output=True, text=True)
        if p.returncode != 0:
            raise RuntimeError("ffmpeg failed: " + (p.stdout + p.stderr)[-800:])

    def ffmpeg_try(self, cmd):
        self.n += 1
        sh = self.work / f"ff{self.n}.sh"
        sh.write_text(" ".join(shlex.quote(a) for a in ["ffmpeg", "-nostdin", "-y", "-loglevel", "error"] + cmd)
                      + "\n", encoding="utf-8", newline="\n")
        p = subprocess.run(WSL + ["--cd", str(ROOT), "-e", "sh", rel(sh)], capture_output=True, text=True)
        return p.returncode == 0, p.stdout + p.stderr

    GDI_PS = r"""
param([string]$jobs)
Add-Type -AssemblyName System.Drawing
$codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
foreach ($line in [System.IO.File]::ReadAllLines($jobs)) {
  $p = $line.Split("`t")
  if ($p[0] -eq 'enc') {
    $bmp = New-Object System.Drawing.Bitmap($p[1])
    $ep = New-Object System.Drawing.Imaging.EncoderParameters(1)
    $ep.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter([System.Drawing.Imaging.Encoder]::Quality, [long]$p[3])
    $bmp.Save($p[2], $codec, $ep)
    $bmp.Dispose()
  } else {
    try {
      $bmp = New-Object System.Drawing.Bitmap($p[1])
      $rect = New-Object System.Drawing.Rectangle(0, 0, $bmp.Width, $bmp.Height)
      $d = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
      $n = $d.Stride * $bmp.Height
      $buf = New-Object byte[] $n
      [System.Runtime.InteropServices.Marshal]::Copy($d.Scan0, $buf, 0, $n)
      $bmp.UnlockBits($d)
      $fs = [System.IO.File]::Create($p[2])
      $hdr = [BitConverter]::GetBytes([int]$bmp.Width)
      $fs.Write($hdr, 0, 4)
      $hdr = [BitConverter]::GetBytes([int]$bmp.Height)
      $fs.Write($hdr, 0, 4)
      $hdr = [BitConverter]::GetBytes([int]$d.Stride)
      $fs.Write($hdr, 0, 4)
      $fs.Write($buf, 0, $n)
      $fs.Close()
      $bmp.Dispose()
    } catch { }
  }
}
"""

    def gdi(self, jobs):
        """jobs: ('enc', src.bmp, dst.jpg, q) / ('dec', src.jpg, dst.raw); absolute paths."""
        if not jobs or not build.IS_WIN_HOST:
            return False
        ps = self.work / "gdi.ps1"
        ps.write_text(self.GDI_PS, encoding="utf-8")
        lst = self.work / "gdi_jobs.txt"
        lst.write_text("\n".join("\t".join(str(x) for x in j) for j in jobs), encoding="utf-8")
        p = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(ps), str(lst)],
                           capture_output=True, text=True)
        if p.returncode != 0:
            raise RuntimeError("GDI+ failed: " + (p.stdout + p.stderr)[-800:])
        return True

    def libjpeg(self, items):
        """items: list of (key, jpeg bytes) -> {key: (w, h, comps, samples) or None}, decoded
        by the system libjpeg-turbo in WSL (default settings: islow IDCT, fancy upsampling)."""
        exe = self.work / "ljdec"
        src = self.work / "ljdec.c"
        if not exe.exists() or not src.exists() or src.read_text() != LJDEC_C:
            src.write_text(LJDEC_C, encoding="utf-8", newline="\n")
            p = subprocess.run(WSL + ["--cd", str(ROOT), "-e", "gcc", "-O1", "-o", rel(exe), rel(src), LIBJPEG_SO],
                               capture_output=True, text=True)
            if p.returncode != 0:
                raise RuntimeError("cannot build the libjpeg helper: " + p.stdout + p.stderr)
        self.n += 1
        lines, outs = [], {}
        for i, (key, data) in enumerate(items):
            f = self.work / f"lj{self.n}_{i}.jpg"
            f.write_bytes(data)
            o = f.with_suffix(".lj")
            if o.exists():
                o.unlink()
            lines.append(f"{rel(f)} {rel(o)}")
            outs[key] = o
        lst = self.work / f"lj{self.n}.txt"
        lst.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        p = subprocess.run(WSL + ["--cd", str(ROOT), "-e", rel(exe), rel(lst)], capture_output=True, text=True)
        if p.returncode != 0:
            raise RuntimeError("libjpeg helper failed: " + p.stdout + p.stderr)
        res = {}
        for key, o in outs.items():
            if not o.exists():
                res[key] = None
                continue
            b = o.read_bytes()
            w, h, n = struct.unpack_from("<III", b)
            res[key] = (w, h, n, np.frombuffer(b, np.uint8, offset=12)[:w * h * n].reshape(h, w, n))
            o.unlink()
            o.with_suffix(".jpg").unlink()
        return res


LIBJPEG_SO = "/usr/lib/x86_64-linux-gnu/libjpeg.so.8"
LJDEC_C = r"""/* ljdec -- decode JPEG files with the system libjpeg(-turbo) for jpeg_check.py
 * (default settings: islow IDCT, fancy upsampling).  No headers needed: the few
 * entry points are declared here and the size of jpeg_decompress_struct is
 * probed through the error handler.  usage: ljdec LIST (lines "in.jpg out.raw");
 * out = u32 w, h, components, then the samples. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <setjmp.h>
extern void *jpeg_std_error(void *err);
extern void jpeg_CreateDecompress(void *cinfo, int version, size_t structsize);
extern void jpeg_stdio_src(void *cinfo, FILE *f);
extern int jpeg_read_header(void *cinfo, int require_image);
extern int jpeg_start_decompress(void *cinfo);
extern unsigned jpeg_read_scanlines(void *cinfo, unsigned char **rows, unsigned max);
extern int jpeg_finish_decompress(void *cinfo);
extern void jpeg_destroy_decompress(void *cinfo);
static jmp_buf jb;
static void on_error(void *c) { (void)c; longjmp(jb, 1); }
static void on_msg(void *c, int lvl) { (void)c; (void)lvl; }
static unsigned char err[4096], cinfo[8192];
static size_t structsize;
static unsigned rd16(const unsigned char *p) { return (p[0] << 8) | p[1]; }
int main(int argc, char **argv)
{
    char in[1024], out[1024];
    FILE *lst;
    if (argc < 2) return 2;
    jpeg_std_error(err);
    *(void **)err = (void *)on_error;
    *(void **)(err + sizeof(void *)) = (void *)on_msg;
    for (structsize = 200; structsize < 4000; structsize += 8) {
        memset(cinfo, 0, sizeof cinfo);
        *(void **)cinfo = err;
        if (setjmp(jb) == 0) { jpeg_CreateDecompress(cinfo, 80, structsize); break; }
    }
    if (structsize >= 4000) return 3;
    jpeg_destroy_decompress(cinfo);
    if (!(lst = fopen(argv[1], "r"))) return 4;
    while (fscanf(lst, "%1023s %1023s", in, out) == 2) {
        FILE *f = fopen(in, "rb"), *o;
        unsigned char *data, *buf = 0;
        long n;
        unsigned w = 0, h = 0, nc = 0, y, i;
        if (!f) continue;
        fseek(f, 0, SEEK_END); n = ftell(f); fseek(f, 0, SEEK_SET);
        data = malloc(n + 1);
        if (fread(data, 1, n, f) != (size_t)n) { fclose(f); free(data); continue; }
        fseek(f, 0, SEEK_SET);
        for (i = 2; i + 9 < (unsigned)n; ) {
            unsigned m = data[i + 1];
            if (data[i] != 0xFF) { i++; continue; }
            if (m == 0xFF || m == 0 || (m >= 0xD0 && m <= 0xD9)) { i += 2; continue; }
            if (m >= 0xC0 && m <= 0xC2) { h = rd16(data + i + 5); w = rd16(data + i + 7); nc = data[i + 9]; break; }
            i += 2 + rd16(data + i + 2);
        }
        free(data);
        memset(cinfo, 0, sizeof cinfo);
        *(void **)cinfo = err;
        if (w && h && setjmp(jb) == 0) {
            jpeg_CreateDecompress(cinfo, 80, structsize);
            jpeg_stdio_src(cinfo, f);
            jpeg_read_header(cinfo, 1);
            jpeg_start_decompress(cinfo);
            buf = calloc((size_t)w * h * 4 + 16, 1);
            for (y = 0; y < h; y++) {
                unsigned char *row = buf + (size_t)y * w * nc;
                if (jpeg_read_scanlines(cinfo, &row, 1) != 1) break;
            }
            jpeg_finish_decompress(cinfo);
            jpeg_destroy_decompress(cinfo);
            if ((o = fopen(out, "wb"))) {
                unsigned hdr[3];
                hdr[0] = w; hdr[1] = h; hdr[2] = nc;
                fwrite(hdr, 4, 3, o);
                fwrite(buf, 1, (size_t)w * h * nc, o);
                fclose(o);
            }
        } else {
            jpeg_destroy_decompress(cinfo);
        }
        free(buf);
        fclose(f);
    }
    fclose(lst);
    return 0;
}
"""


def lj_rgb(res, adobe, o=1):
    """libjpeg output -> expected RGB of our decoder: CMYK converted like jpeg_cmyk_rgb
    (Adobe marker: inverted, else plain), EXIF orientation applied."""
    if res is None:
        return None
    w, h, n, s = res
    s = s.astype(np.int64)
    if n == 1:
        rgb = np.concatenate([s, s, s], -1)
    elif n == 3:
        rgb = s
    else:
        c, k = s[..., :3], s[..., 3:4]
        rgb = div255(c * k) if adobe is not None else div255((255 - c) * (255 - k))
    return orient(rgb, o)


def read_gdi_raw(path):
    b = Path(path).read_bytes()
    w, h, stride = struct.unpack_from("<iii", b)
    px = np.frombuffer(b, np.uint8, offset=12).reshape(h, stride)[:, :w * 4].reshape(h, w, 4)
    return px[..., 2::-1].astype(np.int64)


def parse_jpeg(data):
    """-> dict(sof marker, W, H, comps [(id, h, v, tq)], dqt {id: 64 natural}, jfif, segments)."""
    info = dict(segs=[], dqt={}, comps=[], sof=None, jfif=None, adobe=None)
    p = 2
    while p + 4 <= len(data):
        if data[p] != 0xFF:
            raise ValueError(f"no marker at {p}")
        m = data[p + 1]
        if m == 0xD9:
            info["segs"].append((m, p))
            break
        ln = struct.unpack_from(">H", data, p + 2)[0]
        body = data[p + 4:p + 2 + ln]
        info["segs"].append((m, p))
        if m in (0xC0, 0xC1, 0xC2):
            info["sof"] = m
            _, info["H"], info["W"], n = struct.unpack_from(">BHHB", body)
            info["comps"] = [(body[6 + 3 * i], body[7 + 3 * i] >> 4, body[7 + 3 * i] & 15, body[8 + 3 * i]) for i in range(n)]
        elif m == 0xDB:
            q = 0
            while q < len(body):
                pq, tq = body[q] >> 4, body[q] & 15
                vals = struct.unpack_from(">64H", body, q + 1) if pq else body[q + 1:q + 65]
                nat = [0] * 64
                for k in range(64):
                    nat[ZZ[k]] = vals[k]
                info["dqt"][tq] = nat
                q += 1 + 64 * (pq + 1)
        elif m == 0xE0 and body[:5] == b"JFIF\x00":
            info["jfif"] = body
        elif m == 0xEE and body[:5] == b"Adobe" and len(body) >= 12:
            info["adobe"] = body[11]
        elif m == 0xDA:
            q = p + 2 + ln
            while q < len(data) - 1:
                if data[q] == 0xFF and data[q + 1] != 0 and not (0xD0 <= data[q + 1] <= 0xD7):
                    break
                q += 1
            info["scan_data"] = data[p + 2 + ln:q]
            p = q
            continue
        p += 2 + ln
    return info


def ff_planes_cmd(jpg, raw, pix):
    return ["-idct", "int", "-i", rel(jpg), "-f", "rawvideo", "-pix_fmt", pix, rel(raw)]


PIX = {((1, 1), (1, 1)): "yuvj444p", ((2, 1), (1, 1)): "yuvj422p", ((2, 2), (1, 1)): "yuvj420p",
       ((1, 2), (1, 1)): "yuvj440p", ((4, 1), (1, 1)): "yuvj411p"}


def ff_pix_for(info):
    if len(info["comps"]) == 1:
        return "gray"
    if len(info["comps"]) != 3:
        return None
    y, cb, cr = info["comps"]
    if (cb[1], cb[2]) != (cr[1], cr[2]):
        return None
    hmax, vmax = y[1], y[2]
    if hmax % cb[1] or vmax % cb[2]:
        return None
    return PIX.get(((hmax // cb[1], vmax // cb[2]), (1, 1)))


def model_from_ff_planes(raw, info):
    """ffmpeg's decoded sample planes -> expected RGB via the libjpeg model."""
    W, H = info["W"], info["H"]
    comps = info["comps"]
    b = np.frombuffer(Path(raw).read_bytes(), np.uint8)
    if len(comps) == 1:
        y = b[:W * H].reshape(H, W)
        return model_rgb([y], "gray")
    hmax = max(c[1] for c in comps)
    vmax = max(c[2] for c in comps)
    rows, off = [], 0
    for (_, h, v, _) in comps:
        dw, dh = -(-W * h // hmax), -(-H * v // vmax)
        pl = b[off:off + dw * dh].reshape(dh, dw)
        off += dw * dh
        rows.append(upsample(pl, dw, dh, h, v, hmax, vmax, W, H))
    return model_rgb(rows, "ycc")


# ================================================================ checkers

def expect_exact(exp, tol=0, mean_tol=0.0):
    """exp (H, W, 3) int RGB."""
    h, w, _ = exp.shape

    def chk(res, out):
        if res != 0:
            return f"error {res} ({ERR_NAMES.get(res)})"
        ow, oh = struct.unpack_from("<II", out)
        if (ow, oh) != (w, h):
            return f"size {ow}x{oh}, want {w}x{h}"
        got, a = bgra_to_rgb(out, w, h)
        if not np.all(a == 255):
            return "alpha != 255"
        d = np.abs(got - exp)
        if d.max() > tol or d.mean() > mean_tol:
            y, x = np.unravel_index(np.argmax(d.max(-1)), d.shape[:2])
            return (f"max |d| {d.max()} mean {d.mean():.4f} ({(d.max(-1) > 0).sum()} px), first worst ({x},{y}) "
                    f"got {got[y, x].tolist()} want {exp[y, x].tolist()}")
        return None
    return chk


def expect_close(exp, mean_tol, p99_tol, stats=None, name=""):
    h, w, _ = exp.shape

    def chk(res, out):
        if res != 0:
            return f"error {res} ({ERR_NAMES.get(res)})"
        ow, oh = struct.unpack_from("<II", out)
        if (ow, oh) != (w, h):
            return f"size {ow}x{oh}, want {w}x{h}"
        got, _ = bgra_to_rgb(out, w, h)
        d = np.abs(got - exp)
        p99 = float(np.percentile(d, 99))
        if stats is not None:
            stats.append((name, float(d.mean()), p99, int(d.max())))
        if d.mean() > mean_tol or p99 > p99_tol:
            return f"mean |d| {d.mean():.3f} p99 {p99} max {d.max()}"
        return None
    return chk


def expect_err(*codes):
    def chk(res, out):
        return None if res in codes else f"got {res} ({ERR_NAMES.get(res)}), want {codes}"
    return chk


def expect_dims(w, h):
    def chk(res, out):
        if res != 0:
            return f"error {res} ({ERR_NAMES.get(res)})"
        ow, oh = struct.unpack_from("<II", out)
        return None if (ow, oh) == (w, h) else f"size {ow}x{oh}, want {w}x{h}"
    return chk


def hash_ok(res, out):
    if res == 0:
        if len(out) != 20:
            return "bad hash record"
        w, h, fmt = struct.unpack_from("<III", out)
        if not (1 <= w <= 32768 and 1 <= h <= 32768 and w * h <= 1 << 28):
            return f"bad size {w}x{h}"
        if fmt & 0xFF != 1 or not (fmt >> 8) & 2:
            return f"bad format/flags {fmt:#x}"
        return None
    return None if res in ERR_NAMES and res != 0 else f"bad error code {res}"


def psnr(a, b):
    mse = np.mean((np.asarray(a, np.float64) - np.asarray(b, np.float64)) ** 2)
    return 99.0 if mse == 0 else 10 * math.log10(255 * 255 / mse)


# ================================================================ suite: Python-written files

def model_expect(spec, coefs, qper, cspace, cmode=0, o=1):
    rows = []
    for c, g in enumerate(spec.geo):
        pl = planes_from_coefs(coefs[c], qper[c])
        rows.append(upsample(pl, g["dw"], g["dh"], g["h"], g["v"], spec.hmax, spec.vmax, spec.W, spec.H))
    return orient(model_rgb(rows, cspace, cmode), o)


def make_case(rng, W, H, samp, cspace="ycc", quality=75, ids=None, seed=0):
    spec = JpegSpec(W, H, samp, ids)
    rgb = photo(rng, W, H, seed)
    if cspace == "ycc":
        full = rgb_to_ycc(rgb)
    elif cspace == "gray":
        full = [rgb_to_ycc(rgb)[0]]
    elif cspace == "rgb":
        full = [rgb[..., k] for k in range(3)]
    else:                                   # cmyk / ycck: 4 synthetic planes
        full = [rgb[..., 0], rgb[..., 1], rgb[..., 2], 255 - rgb.mean(-1) / 2]
    planes = comp_planes_from(spec, full)
    qtabs = {0: scaled_q(STD_QL, quality), 1: scaled_q(STD_QC, quality)}
    qsel = [0] + [1] * (spec.nc - 1)
    coefs = [fdct_quant(p, qtabs[qsel[c]]) for c, p in enumerate(planes)]
    return spec, coefs, qtabs, qsel


SAMPLINGS = {
    "444": [(1, 1), (1, 1), (1, 1)], "422": [(2, 1), (1, 1), (1, 1)], "420": [(2, 2), (1, 1), (1, 1)],
    "440": [(1, 2), (1, 1), (1, 1)], "411": [(4, 1), (1, 1), (1, 1)], "410": [(4, 2), (1, 1), (1, 1)],
    "h3v1": [(3, 1), (1, 1), (1, 1)], "h3v2": [(3, 2), (1, 1), (1, 1)], "y11c22": [(1, 1), (2, 2), (2, 2)],
    "mixed": [(2, 2), (2, 1), (1, 2)], "h1v4": [(1, 4), (1, 1), (1, 1)], "h2v2c12": [(2, 2), (1, 2), (1, 2)],
    "h4v4": [(4, 4), (1, 1), (1, 1)], "h3v3c21": [(3, 3), (2, 1), (1, 1)],
}
SIZES = [(1, 1), (2, 3), (4, 5), (5, 2), (7, 9), (17, 33), (33, 17), (64, 48), (101, 37)]


def suite_pydec(rng, cases_out):
    """fills cases_out with (name, file, expected RGB, orientation); -> writer check list"""
    ffcheck = []            # (name, jpg bytes, spec, coefs, qper) for the writer sanity check

    def add(name, data, exp, spec=None, coefs=None, qper=None, ff=False, o=1):
        cases_out.append((name, data, exp, o))
        if ff:
            ffcheck.append((name, data, spec, coefs, qper))

    # grey, every size, standard and optimised tables
    for (w, h) in SIZES + [(256, 3), (3, 256)]:
        for hf in ("std", "opt"):
            spec, coefs, qt, qs = make_case(rng, w, h, [(1, 1)], "gray", 80)
            data = write_jpeg(spec, coefs, qt, qs, huff=hf)
            add(f"gray {w}x{h} {hf}", data, model_expect(spec, coefs, [qt[0]], "gray"), spec, coefs, [qt[0]], hf == "opt")
    # grey with sampling factors 2x2 (a lone component is never subsampled)
    spec, coefs, qt, qs = make_case(rng, 45, 29, [(2, 2)], "gray", 60)
    add("gray h2v2 45x29", write_jpeg(spec, coefs, qt, qs), model_expect(spec, coefs, [qt[0]], "gray"))
    # YCbCr, every sampling layout and size
    for name, samp in SAMPLINGS.items():
        sizes = SIZES if name in ("444", "422", "420", "440", "411") else [(1, 1), (17, 33), (101, 37)]
        for (w, h) in sizes:
            spec, coefs, qt, qs = make_case(rng, w, h, samp, "ycc", 85)
            blocks = sum(a * b for a, b in samp)
            scans = None if blocks <= 10 else [([c], 0, 63, 0, 0) for c in range(3)]
            data = write_jpeg(spec, coefs, qt, qs, scans=scans)
            qper = [qt[q] for q in qs]
            add(f"ycc {name} {w}x{h}", data, model_expect(spec, coefs, qper, "ycc"), spec, coefs, qper,
                name in ("444", "422", "420", "440", "411") and w > 1)
    # quality range
    for q in (1, 10, 50, 95, 100):
        spec, coefs, qt, qs = make_case(rng, 40, 24, SAMPLINGS["420"], "ycc", q)
        qper = [qt[x] for x in qs]
        add(f"ycc 420 q{q}", write_jpeg(spec, coefs, qt, qs), model_expect(spec, coefs, qper, "ycc"))
    # non-interleaved sequential scans, DQT for chroma only before its scan
    spec, coefs, qt, qs = make_case(rng, 50, 30, SAMPLINGS["420"], "ycc", 70)
    qper = [qt[x] for x in qs]
    exp = model_expect(spec, coefs, qper, "ycc")
    add("ycc 420 3 scans late DQT", write_jpeg(spec, coefs, qt, qs, scans=[([c], 0, 63, 0, 0) for c in range(3)],
                                               late_dqt=True), exp)
    add("ycc 420 scans Y+CbCr", write_jpeg(spec, coefs, qt, qs, scans=[([0], 0, 63, 0, 0), ([1, 2], 0, 63, 0, 0)]), exp)
    # SOF1 with table slots 2 and 3, 16-bit DQT, MJPEG without DHT
    add("ycc 420 SOF1 tables 2/3", write_jpeg(spec, coefs, qt, qs, mode="ext", tsel=[(0, 0), (2, 3), (2, 3)]), exp)
    qt16 = {0: [min(65535, v * 3) for v in qt[0]], 1: [v * 5 for v in qt[1]]}
    spec2, coefs2, _, _ = make_case(rng, 50, 30, SAMPLINGS["420"], "ycc", 70)
    coefs2 = [np.round(c * np.asarray(qt[q]) / np.asarray(qt16[q])).astype(np.int64) for c, q in zip(coefs2, qs)]
    add("ycc 420 16-bit DQT", write_jpeg(spec2, coefs2, qt16, qs, mode="ext", q16=True),
        model_expect(spec2, coefs2, [qt16[x] for x in qs], "ycc"))
    add("ycc 420 no DHT (MJPEG)", write_jpeg(spec, coefs, qt, qs, huff="none"), exp)
    add("ycc 420 std tables", write_jpeg(spec, coefs, qt, qs, huff="std"), exp)
    # structure oddities
    add("fill bytes", write_jpeg(spec, coefs, qt, qs, fill=True), exp)
    add("garbage between segments", write_jpeg(spec, coefs, qt, qs, garbage=True), exp)
    add("APP2/APP13/COM", write_jpeg(spec, coefs, qt, qs, extra_app=True), exp)
    add("no EOI", write_jpeg(spec, coefs, qt, qs, eoi=False), exp)
    add("data after EOI", write_jpeg(spec, coefs, qt, qs, trailing=b"\xFF\xD8\xFF\xE0junk" + bytes(50)), exp)
    add("no JFIF", write_jpeg(spec, coefs, qt, qs, jfif=False, com=False), exp)
    # restart intervals
    for ri in (1, 2, 3, 7):
        add(f"restart {ri}", write_jpeg(spec, coefs, qt, qs, restart=ri), exp)
    add("restart 2 non-interleaved", write_jpeg(spec, coefs, qt, qs, restart=2,
                                                 scans=[([c], 0, 63, 0, 0) for c in range(3)]), exp)
    spec3, coefs3, qt3, qs3 = make_case(rng, 150, 70, SAMPLINGS["422"], "ycc", 90)
    qper3 = [qt3[x] for x in qs3]
    add("restart 5 422 150x70", write_jpeg(spec3, coefs3, qt3, qs3, restart=5), model_expect(spec3, coefs3, qper3, "ycc"))
    # progressive
    for name in ("420", "444", "422"):
        for (w, h) in [(1, 1), (17, 33), (64, 48), (101, 37)]:
            sp, cf, qt4, qs4 = make_case(rng, w, h, SAMPLINGS[name], "ycc", 85)
            qp = [qt4[x] for x in qs4]
            add(f"prog {name} {w}x{h}", write_jpeg(sp, cf, qt4, qs4, mode="prog"), model_expect(sp, cf, qp, "ycc"),
                sp, cf, qp, w > 1)
    sp, cf, qt4, qs4 = make_case(rng, 77, 45, SAMPLINGS["420"], "ycc", 95)
    qp = [qt4[x] for x in qs4]
    ex = model_expect(sp, cf, qp, "ycc")
    add("prog spectral only", write_jpeg(sp, cf, qt4, qs4, mode="prog", scans=default_script(3, sa=False)), ex, sp, cf, qp, True)
    add("prog generic SA script", write_jpeg(sp, cf, qt4, qs4, mode="prog", scans=default_script(3, sa=True)[:0] +
                                             [([0, 1, 2], 0, 0, 0, 2), ([0, 1, 2], 0, 0, 2, 1)] +
                                             [([c], 1, 63, 0, 3) for c in range(3)] +
                                             [([c], 1, 63, 3, 2) for c in range(3)] +
                                             [([c], 1, 63, 2, 1) for c in range(3)] +
                                             [([0, 1, 2], 0, 0, 1, 0)] + [([c], 1, 63, 1, 0) for c in range(3)]), ex)
    add("prog DC per component", write_jpeg(sp, cf, qt4, qs4, mode="prog",
                                            scans=[([c], 0, 0, 0, 0) for c in range(3)] + [([c], 1, 63, 0, 0) for c in range(3)]), ex)
    add("prog restart 3", write_jpeg(sp, cf, qt4, qs4, mode="prog", restart=3), ex)
    add("prog restart 1", write_jpeg(sp, cf, qt4, qs4, mode="prog", restart=1), ex)
    sp, cf, qt4, qs4 = make_case(rng, 70, 50, [(1, 1)], "gray", 90)
    add("prog gray 70x50", write_jpeg(sp, cf, qt4, qs4, mode="prog"), model_expect(sp, cf, [qt4[0]], "gray"),
        sp, cf, [qt4[0]], True)
    sp, cf, qt4, qs4 = make_case(rng, 300, 200, SAMPLINGS["420"], "ycc", 92)
    qp = [qt4[x] for x in qs4]
    add("prog 420 300x200", write_jpeg(sp, cf, qt4, qs4, mode="prog"), model_expect(sp, cf, qp, "ycc"), sp, cf, qp, True)
    # colour spaces
    for (w, h) in [(1, 1), (23, 17), (64, 40)]:
        sp, cf, qt5, qs5 = make_case(rng, w, h, SAMPLINGS["444"], "rgb", 90)
        qp = [qt5[x] for x in qs5]
        ex = model_expect(sp, cf, qp, "rgb")
        add(f"Adobe RGB {w}x{h}", write_jpeg(sp, cf, qt5, qs5, jfif=False, adobe=0), ex)
        sp2 = JpegSpec(w, h, SAMPLINGS["444"], ids=[82, 71, 66])
        add(f"'RGB' ids {w}x{h}", write_jpeg(sp2, cf, qt5, qs5, jfif=False), ex)
        add(f"JFIF wins over 'RGB' ids {w}x{h}", write_jpeg(sp2, cf, qt5, qs5, jfif=True), model_expect(sp, cf, qp, "ycc"))
        samp4 = [(1, 1)] * 4
        sp, cf, qt5, qs5 = make_case(rng, w, h, samp4, "cmyk", 90)
        qp = [qt5[x] for x in qs5]
        add(f"Adobe CMYK {w}x{h}", write_jpeg(sp, cf, qt5, qs5, jfif=False, adobe=0), model_expect(sp, cf, qp, "cmyk", 0))
        add(f"plain CMYK {w}x{h}", write_jpeg(sp, cf, qt5, qs5, jfif=False), model_expect(sp, cf, qp, "cmyk", 1))
        add(f"Adobe YCCK {w}x{h}", write_jpeg(sp, cf, qt5, qs5, jfif=False, adobe=2), model_expect(sp, cf, qp, "ycck"))
        samp4 = [(2, 2), (1, 1), (1, 1), (2, 2)]
        sp, cf, qt5, qs5 = make_case(rng, w, h, samp4, "cmyk", 80)
        qp = [qt5[x] for x in qs5]
        add(f"Adobe YCCK 420 {w}x{h}", write_jpeg(sp, cf, qt5, qs5, jfif=False, adobe=2), model_expect(sp, cf, qp, "ycck"))
        add(f"Adobe YCCK 420 prog {w}x{h}", write_jpeg(sp, cf, qt5, qs5, jfif=False, adobe=2, mode="prog"),
            model_expect(sp, cf, qp, "ycck"))
    # EXIF orientation
    sp, cf, qt6, qs6 = make_case(rng, 21, 13, SAMPLINGS["420"], "ycc", 85)
    qp = [qt6[x] for x in qs6]
    for o in range(1, 9):
        for big in (False, True):
            add(f"EXIF orientation {o} {'MM' if big else 'II'}",
                write_jpeg(sp, cf, qt6, qs6, exif=(o, big)), model_expect(sp, cf, qp, "ycc", o=o), o=o)
    add("EXIF orientation 6 prog", write_jpeg(sp, cf, qt6, qs6, exif=6, mode="prog"), model_expect(sp, cf, qp, "ycc", o=6),
        o=6)
    add("EXIF orientation 9 (invalid) ignored", write_jpeg(sp, cf, qt6, qs6, exif=9), model_expect(sp, cf, qp, "ycc"))
    sp, cf, qt6, qs6 = make_case(rng, 1, 5, [(1, 1)], "gray", 85)
    add("EXIF orientation 8 gray 1x5", write_jpeg(sp, cf, qt6, qs6, exif=8), model_expect(sp, cf, [qt6[0]], "gray", o=8),
        o=8)
    # a larger 4:2:0 image with restarts
    sp, cf, qt7, qs7 = make_case(rng, 640, 480, SAMPLINGS["420"], "ycc", 88)
    qp = [qt7[x] for x in qs7]
    add("ycc 420 640x480 restart 40", write_jpeg(sp, cf, qt7, qs7, restart=40), model_expect(sp, cf, qp, "ycc"))
    return ffcheck


def pydec_recs(ext, cases, suite_lj):
    """decode records (expected = model); the model itself is checked against libjpeg-turbo."""
    lj = ext.libjpeg([(i, c[1]) for i, c in enumerate(cases)])
    recs = []
    for i, (name, data, exp, o) in enumerate(cases):
        ref = lj_rgb(lj[i], True if data.find(b"\xFF\xEE") >= 0 and b"Adobe" in data else None, o)
        if ref is None:
            suite_lj.notes.append(f"libjpeg cannot decode: {name}")
        elif ref.shape != exp.shape:
            suite_lj.fail(f"{name}: libjpeg size {ref.shape} vs model {exp.shape}")
        elif np.abs(ref - exp).max():
            d = np.abs(ref - exp)
            suite_lj.fail(f"{name}: model differs from libjpeg (max {d.max()}, {(d.max(-1) > 0).sum()} px)")
        else:
            suite_lj.ok()
        recs.append(Rec(OP_JDEC, 0, data, expect_exact(exp), name))
    return recs


def suite_writer_ff(ext, ffcheck, suite):
    """ffmpeg must decode the writer's files to the model's sample planes."""
    cmds, jobs = [], []
    for i, (name, data, spec, coefs, qper) in enumerate(ffcheck):
        jpg = ext.path(f"w{i}.jpg")
        raw = ext.path(f"w{i}.raw")
        jpg.write_bytes(data)
        info = parse_jpeg(data)
        pix = ff_pix_for(info)
        if pix is None:
            continue
        cmds.append(ff_planes_cmd(jpg, raw, pix))
        jobs.append((name, raw, spec, coefs, qper, info))
    ext.ffmpeg(cmds)
    for name, raw, spec, coefs, qper, info in jobs:
        b = np.frombuffer(raw.read_bytes(), np.uint8)
        off, bad = 0, None
        for c, g in enumerate(spec.geo):
            pl = b[off:off + g["dw"] * g["dh"]].reshape(g["dh"], g["dw"]).astype(int)
            off += g["dw"] * g["dh"]
            ref = planes_from_coefs(coefs[c], qper[c])[:g["dh"], :g["dw"]].astype(int)
            d = np.abs(pl - ref).max()
            if d > 1:
                bad = f"component {c}: ffmpeg planes differ from the model by up to {d}"
        if bad:
            suite.fail(f"writer/ffmpeg {name}: {bad}")
        else:
            suite.ok()


# ================================================================ suite: ffmpeg / GDI+ encoded files

# (pix_fmt, -q:v, slices); ffmpeg's MJPEG encoder writes 4:4:4 / 4:2:2 / 4:2:0 only (no grey),
# with -slices > 1 it adds a restart interval (DRI + RSTn between the slices)
FF_VARIANTS = [("yuvj444p", 2, 1), ("yuvj444p", 10, 1), ("yuvj422p", 3, 1), ("yuvj420p", 2, 1), ("yuvj420p", 5, 1),
               ("yuvj420p", 20, 1), ("yuvj420p", 31, 1), ("yuvj420p", 3, 4), ("yuvj444p", 6, 3), ("yuvj422p", 4, 2)]
FF_SIZES = [(1, 1), (2, 2), (7, 9), (17, 33), (33, 17), (97, 61), (320, 200)]


def suite_ffdec(rng, ext, cases_out):
    """-> list of Rec; ffmpeg encodes photo-like images in many variants."""
    cmds, files = [], []
    for si, (w, h) in enumerate(FF_SIZES):
        img = photo(rng, w, h, si)
        src = ext.path(f"ff_src{si}.ppm")
        src.write_bytes(ppm(img))
        for vi, (pix, q, sl) in enumerate(FF_VARIANTS):
            for hf in ("default", "optimal"):
                jpg = ext.path(f"ff_{si}_{vi}_{hf}.jpg")
                extra = ["-slices", str(sl)] if sl > 1 else []
                cmds.append(["-i", rel(src), "-pix_fmt", pix, "-q:v", str(q), "-huffman", hf] + extra + [rel(jpg)])
                files.append((f"ffmpeg {pix} q{q} {hf}{f' slices {sl}' if sl > 1 else ''} {w}x{h}", jpg))
    ext.ffmpeg(cmds)
    cmds, work = [], []
    for name, jpg in files:
        data = jpg.read_bytes()
        info = parse_jpeg(data)
        rgb = jpg.with_suffix(".rgb")
        cmds.append(["-i", rel(jpg), "-f", "rawvideo", "-pix_fmt", "rgb24", rel(rgb)])
        work.append((name, data, info, rgb))
    ext.ffmpeg(cmds)
    lj = ext.libjpeg([(name, data) for name, data, info, rgb in work])
    recs, stats, notes = [], [], []
    for name, data, info, rgb in work:
        exp = lj_rgb(lj[name], None)
        recs.append(Rec(OP_JDEC, 0, data, expect_exact(exp), name + " vs libjpeg"))
        if info["W"] * info["H"] >= 97 * 61:
            ffrgb = np.frombuffer(rgb.read_bytes(), np.uint8).reshape(info["H"], info["W"], 3).astype(np.int64)
            # swscale converts / upsamples differently: a sanity check only (a wrong colour
            # space, layout or orientation gives mean errors far above these limits)
            tol = (1.0, 4) if "444" in name else (6.0, 64)
            recs.append(Rec(OP_JDEC, 0, data, expect_close(ffrgb, tol[0], tol[1], stats, name), name + " vs ffmpeg rgb"))
        cases_out.append((name, data))
        if "slices" in name and w_h_ok(info) and b"\xFF\xDD" not in data:
            recs.append(Rec(OP_JDEC, 0, data, lambda res, out: "no restart interval in a sliced file", name + " DRI"))
        if "default" in name and "320x200" in name and "420" in name and "q2 " in name:
            dht = parse_dht(data)
            ok = all(dht.get(k) == (list(b), list(v)) for k, (b, v) in STD_HUFF.items())
            notes.append("ffmpeg -huffman default DHT == Annex K tables: " + ("yes" if ok else "NO"))
            if not ok:
                recs.append(Rec(OP_JDEC, 0, data, lambda res, out: "Annex K tables differ from ffmpeg's", "Annex K tables"))
    return recs, stats, notes


def w_h_ok(info):
    return info["W"] >= 16 and info["H"] >= 32


def parse_dht(data):
    """-> {(class, id): (bits, vals)} of all DHT segments"""
    out, p = {}, 2
    while True:
        p = data.find(b"\xFF\xC4", p)
        if p < 0:
            return out
        ln = struct.unpack_from(">H", data, p + 2)[0]
        body, q = data[p + 4:p + 2 + ln], 0
        while q + 17 <= len(body):
            n = sum(body[q + 1:q + 17])
            out[(body[q] >> 4, body[q] & 15)] = (list(body[q + 1:q + 17]), list(body[q + 17:q + 17 + n]))
            q += 17 + n
        p += 2 + ln


GDI_Q = [5, 25, 50, 75, 90, 100]
GDI_SIZES = [(1, 1), (7, 9), (17, 33), (100, 37), (320, 240)]


def suite_gdidec(rng, ext, cases_out):
    jobs, files = [], []
    for si, (w, h) in enumerate(GDI_SIZES):
        img = photo(rng, w, h, 100 + si)
        src = ext.path(f"gdi_src{si}.bmp")
        src.write_bytes(bmp24(img))
        for q in GDI_Q:
            jpg = ext.path(f"gdi_{si}_q{q}.jpg")
            jobs.append(("enc", src.resolve(), jpg.resolve(), q))
            files.append((f"GDI+ q{q} {w}x{h}", jpg))
    if not ext.gdi(jobs):
        return [], []
    jobs = [("dec", jpg.resolve(), jpg.with_suffix(".gdi").resolve()) for _, jpg in files]
    ext.gdi(jobs)
    work = [(name, jpg.read_bytes(), jpg.with_suffix(".gdi")) for name, jpg in files]
    lj = ext.libjpeg([(name, data) for name, data, gdi in work])
    recs, stats, notes = [], [], []
    subs = set()
    for name, data, gdi in work:
        info = parse_jpeg(data)
        subs.add(str([(c[1], c[2]) for c in info["comps"]]))
        recs.append(Rec(OP_JDEC, 0, data, expect_exact(lj_rgb(lj[name], None)), name + " vs libjpeg"))
        recs.append(Rec(OP_JDEC, 0, data, expect_close(read_gdi_raw(gdi), 0.5, 2, stats, name), name + " vs GDI+"))
        cases_out.append((name, data))
    notes.append("GDI+ sampling factors: " + ", ".join(sorted(subs)))
    return recs, stats, notes


# ================================================================ suite: encoder

def make_surface(rng, w, h, fmt, alpha):
    """-> (record data, source RGB composited over white (H, W, 3) or grey (H, W))."""
    r = np.random.default_rng(rng.randrange(1 << 30))
    if fmt == 2:
        g = photo(rng, w, h)[..., 1]
        return struct.pack("<III", w, h, 2) + g.tobytes(), g.astype(np.int64)
    rgb = photo(rng, w, h).astype(np.int64)
    if fmt == 0:
        a = r.integers(0, 256, (h, w))                          # garbage alpha byte
        bgra = np.concatenate([rgb[..., ::-1], a[..., None]], -1)
        return struct.pack("<III", w, h, 0) + bgra.astype(np.uint8).tobytes(), rgb
    if alpha == "opaque":
        a = np.full((h, w), 255)
    elif alpha == "random":
        a = r.integers(0, 256, (h, w))
    else:
        a = np.where(r.random((h, w)) < 0.7, 255, r.integers(0, 256, (h, w)))
    pm = (rgb * a[..., None] + 127) // 255
    over_white = pm + (255 - a)[..., None]
    bgra = np.concatenate([pm[..., ::-1], a[..., None]], -1)
    return struct.pack("<III", w, h, 1) + bgra.astype(np.uint8).tobytes(), over_white


def check_structure(data, w, h, fmt, q):
    info = parse_jpeg(data)
    if data[:2] != b"\xFF\xD8" or data[-2:] != b"\xFF\xD9":
        return "SOI / EOI missing"
    if info["sof"] != 0xC0:
        return f"SOF {info['sof']}"
    if (info["W"], info["H"]) != (w, h):
        return f"SOF size {info['W']}x{info['H']}"
    j = info["jfif"]
    if not j or j[5:7] != b"\x01\x01" or j[7] != 1 or struct.unpack(">HH", j[8:12]) != (96, 96):
        return "JFIF APP0 (1.01, 96 dpi) missing"
    nc = 1 if fmt == 2 else 3
    if len(info["comps"]) != nc:
        return f"{len(info['comps'])} components"
    ys = (info["comps"][0][1], info["comps"][0][2])
    want = (1, 1) if nc == 1 or q >= 90 else (2, 2)
    if ys != want or any((c[1], c[2]) != (1, 1) for c in info["comps"][1:]):
        return f"sampling {[(c[1], c[2]) for c in info['comps']]}"
    qq = max(1, min(100, q))
    if info["dqt"].get(0) != scaled_q(STD_QL, qq) or (nc == 3 and info["dqt"].get(1) != scaled_q(STD_QC, qq)):
        return "DQT differs from libjpeg's scaled tables"
    sd = info.get("scan_data", b"")
    for i in range(len(sd) - 1):
        if sd[i] == 0xFF and sd[i + 1] != 0:
            return "unstuffed 0xFF in the entropy-coded data"
    return None


ENC_SIZES = [(1, 1), (2, 3), (5, 1), (1, 7), (7, 9), (16, 16), (17, 5), (33, 17), (100, 37), (257, 3)]


def suite_enc(rng, ext, runners, suite, notes):
    """Encodes on every target, checks everything; returns nothing (fills suite)."""
    cases = []
    for (w, h) in ENC_SIZES:
        for fmt, al in ((0, ""), (1, "opaque"), (1, "random"), (1, "mixed"), (2, "")):
            for q in (75, 95):
                cases.append((w, h, fmt, al, q))
    for q in (-5, 1, 10, 30, 50, 60, 85, 89, 90, 99, 100, 150):
        cases.append((97, 61, 1, "opaque", q))
    for q in (50, 75, 89, 90, 95):
        cases.append((640, 360, 1, "opaque", q))
        cases.append((640, 360, 2, "", q))
    items = []
    for (w, h, fmt, al, q) in cases:
        data, src = make_surface(rng, w, h, fmt, al)
        items.append((f"enc {w}x{h} fmt{fmt} {al} q{q}", w, h, fmt, q, data, src))
    outs = {}
    for t, runner in runners.items():
        crashes = []
        res = runner.batch([Rec(OP_JENC, q & 0xFFFFFFFF, data, None, name)
                            for name, w, h, fmt, q, data, src in items], crashes)
        outs[t] = res
        for c in crashes[:5]:
            suite.msgs.append("crash detail: " + c)
    targets = list(runners)
    files = []
    for i, (name, w, h, fmt, q, data, src) in enumerate(items):
        rs = [outs[t][i] for t in targets]
        if any(r is None for r in rs):
            suite.fail(f"CRASH {name}")
            continue
        if any(r[0] != 0 for r in rs):
            suite.fail(f"{name}: encode failed {[r[0] for r in rs]}")
            continue
        jpg = rs[0][1]
        if any(r[1] != jpg for r in rs[1:]):
            suite.fail(f"{name}: output differs between targets")
            continue
        msg = check_structure(jpg, w, h, fmt, q)
        if msg:
            suite.fail(f"{name}: {msg}")
            continue
        suite.ok()
        files.append((i, name, w, h, fmt, q, jpg, src))
    # decode: libjpeg (bit-exact vs our decoder), ffmpeg RGB, GDI+, our decoder
    cmds, gjobs = [], []
    lj = ext.libjpeg([(i, jpg) for i, name, w, h, fmt, q, jpg, src in files])
    for i, name, w, h, fmt, q, jpg, src in files:
        p = ext.path(f"enc{i}.jpg")
        p.write_bytes(jpg)
        cmds.append(["-i", rel(p), "-f", "rawvideo", "-pix_fmt", "rgb24", rel(p.with_suffix(".rgb"))])
        gjobs.append(("dec", p.resolve(), p.with_suffix(".gdi").resolve()))
    ext.ffmpeg(cmds)
    have_gdi = ext.gdi(gjobs)
    thresholds = {50: 30.0, 75: 32.0, 89: 34.0, 90: 36.0, 95: 37.0}
    for t, runner in runners.items():
        recs = []
        for i, name, w, h, fmt, q, jpg, src in files:
            p = ext.path(f"enc{i}.jpg")
            exp = lj_rgb(lj[i], None)
            if exp is None:
                suite.fail(f"{name}: libjpeg cannot decode our file")
                continue
            recs.append(Rec(OP_JDEC, 0, jpg, expect_exact(exp), f"{name} own decode vs libjpeg"))
            srgb = src if src.ndim == 3 else np.stack([src] * 3, -1)
            ffrgb = np.frombuffer(p.with_suffix(".rgb").read_bytes(), np.uint8).reshape(h, w, 3)
            gd = read_gdi_raw(p.with_suffix(".gdi")) if have_gdi and p.with_suffix(".gdi").exists() else None

            def chk(res, out, srgb=srgb, ffrgb=ffrgb, gd=gd, q=q, w=w, h=h, name=name):
                if res != 0:
                    return f"decode error {res}"
                got, _ = bgra_to_rgb(out, w, h)
                ours, ff = psnr(got, srgb), psnr(ffrgb, srgb)
                gdi = psnr(gd, srgb) if gd is not None else None
                if w * h >= 640 * 360 and t == targets[0]:
                    notes.append(f"{name}: PSNR own decoder {ours:.2f} dB, ffmpeg {ff:.2f} dB"
                                 + (f", GDI+ {gdi:.2f} dB" if gdi is not None else ""))
                lim = thresholds.get(q) if w * h >= 640 * 360 else None
                if lim and min(ours, ff, gdi if gdi is not None else 99) < lim:
                    return f"PSNR below {lim} dB: own {ours:.2f}, ffmpeg {ff:.2f}, GDI+ {gdi}"
                # GDI+ decodes like libjpeg (identical PSNR expected); ffmpeg upsamples chroma with
                # swscale, which loses a few dB on tiny images
                if (gdi is not None and abs(ours - gdi) > 0.5) or ff < min(ours - 8, 30):
                    return f"decoders disagree: own {ours:.2f}, ffmpeg {ff:.2f}, GDI+ {gdi}"
                return None
            recs.append(Rec(OP_JDEC, 0, jpg, chk, f"{name} PSNR"))
        crashes = []
        res = runner.batch(recs, crashes)
        for r, rr in zip(recs, res):
            if rr is None:
                suite.fail(f"[{t}] CRASH {r.name}")
                continue
            m = r.check(rr[0], rr[1])
            if m:
                suite.fail(f"[{t}] {r.name}: {m}")
            else:
                suite.ok()
    # relative quality: GDI+ at the same quality setting
    if have_gdi:
        rng2 = random.Random(77)
        img = photo(rng2, 640, 360)
        src = ext.path("encq_src.bmp")
        src.write_bytes(bmp24(img))
        data = struct.pack("<III", 640, 360, 0) + np.concatenate(
            [img[..., ::-1], np.full((360, 640, 1), 255, np.uint8)], -1).tobytes()
        jobs, recs = [], []
        for q in (50, 75, 90, 95):
            jobs.append(("enc", src.resolve(), ext.path(f"encq{q}.jpg").resolve(), q))
            recs.append(Rec(OP_JENC, q, data, None, f"q{q}"))
        ext.gdi(jobs)
        ext.gdi([("dec", ext.path(f"encq{q}.jpg").resolve(), ext.path(f"encq{q}.gdi").resolve()) for q in (50, 75, 90, 95)])
        res = runners[targets[0]].batch(recs, [])
        for q, rr in zip((50, 75, 90, 95), res):
            ours = rr[1]
            p = ext.path(f"encq{q}_own.jpg")
            p.write_bytes(ours)
            ext.gdi([("dec", p.resolve(), p.with_suffix(".gdi").resolve())])
            po = psnr(read_gdi_raw(p.with_suffix(".gdi")), img)
            pg = psnr(read_gdi_raw(ext.path(f"encq{q}.gdi")), img)
            sg = len(ext.path(f"encq{q}.jpg").read_bytes())
            notes.append(f"q{q} 640x360: ours {len(ours):6d} B {po:.2f} dB  |  GDI+ {sg:6d} B {pg:.2f} dB (both decoded by GDI+)")
            if po < pg - 0.5:
                suite.fail(f"q{q}: PSNR {po:.2f} dB well below GDI+ {pg:.2f} dB")
            else:
                suite.ok()


# ================================================================ suite: errors

def suite_errors(rng):
    recs = []
    spec, coefs, qt, qs = make_case(rng, 24, 16, SAMPLINGS["420"], "ycc", 75)
    good = write_jpeg(spec, coefs, qt, qs)
    info = parse_jpeg(good)
    sof_pos = [p for m, p in info["segs"] if m == 0xC0][0]
    sos_pos = [p for m, p in info["segs"] if m == 0xDA][0]

    def patch(data, pos, b):
        return data[:pos] + bytes(b) + data[pos + len(b):]

    def add(name, data, *codes):
        recs.append(Rec(OP_JDEC, 0, data, expect_err(*codes), name))
    add("empty", b"", -1)
    add("1 byte", b"\xFF", -1)
    add("SOI only", b"\xFF\xD8", -1)
    add("SOI EOI", b"\xFF\xD8\xFF\xD9", -2)
    add("PNG signature", b"\x89PNG\r\n\x1a\n" + bytes(30), -2)
    add("no SOI", good[2:], -2)
    add("two SOI", good[:2] + good, -2)
    add("header only", good[:sos_pos], -1)
    add("cut inside SOF", good[:sof_pos + 7], -1)
    for m, nm in ((0xC3, "lossless SOF3"), (0xC5, "differential SOF5"), (0xC9, "arithmetic SOF9"),
                  (0xCA, "arithmetic SOF10"), (0xCB, "arithmetic SOF11"), (0xCD, "SOF13"), (0xCF, "SOF15")):
        add(nm, patch(good, sof_pos + 1, [m]), -9)
    add("DAC segment", good[:sof_pos] + seg(0xCC, b"\x00\x10") + good[sof_pos:], -9)
    add("12-bit precision", patch(good, sof_pos + 4, [12]), -9)
    add("height 0 (DNL)", patch(good, sof_pos + 5, [0, 0]), -9)
    add("width 0", patch(good, sof_pos + 7, [0, 0]), -2)
    add("width 40000", patch(good, sof_pos + 7, struct.pack(">H", 40000)), -12)
    add("20000 x 20000", patch(patch(good, sof_pos + 7, struct.pack(">H", 20000)), sof_pos + 5,
                               struct.pack(">H", 20000)), -12)
    add("sampling factor 0", patch(good, sof_pos + 11, [0x02]), -2)
    add("sampling factor 5", patch(good, sof_pos + 11, [0x51]), -2)
    add("quant table 4", patch(good, sof_pos + 12, [4]), -2)
    add("undefined quant table", patch(good, sof_pos + 12, [3]), -2)
    add("SOS before SOF", good[:2] + good[sos_pos:], -2)
    add("SOS with unknown component", patch(good, sos_pos + 5, [9]), -2)
    add("SOS table 4", patch(good, sos_pos + 6, [0x40]), -2)
    add("second SOF", good[:sos_pos] + good[sof_pos:sos_pos] + good[sos_pos:], -2)
    add("segment length 1", patch(good, sof_pos + 2, [0, 1]), -2)
    # 2 components
    sp2 = JpegSpec(8, 8, [(1, 1), (1, 1)])
    cf2 = [np.zeros((1, 1, 64), np.int64)] * 2
    add("2 components", write_jpeg(sp2, cf2, qt, [0, 1]), -9)
    # progressive with bad spectral selection
    sp3, cf3, qt3, qs3 = make_case(rng, 16, 16, [(1, 1)], "gray", 75)
    add("progressive Se < Ss", write_jpeg(sp3, cf3, qt3, qs3, mode="prog", scans=[([0], 0, 0, 0, 0), ([0], 5, 3, 0, 0)]), -2)
    add("progressive DC with Se > 0", write_jpeg(sp3, cf3, qt3, qs3, mode="prog", scans=[([0], 0, 5, 0, 0)]), -2)
    # over-subscribed Huffman table (3 codes of length 1) used by the scan
    bad = bytes([0x00, 3] + [0] * 15 + [0, 1, 2])
    d = write_jpeg(sp3, cf3, qt3, qs3, huff="std")
    pos = d.index(b"\xFF\xDA")
    add("over-subscribed DHT", d[:pos] + seg(0xC4, bad) + d[pos:], -4)
    add("unused over-subscribed DHT is fine", d[:pos] + seg(0xC4, bytes([0x03, 3] + [0] * 15 + [0, 1, 2])) + d[pos:], 0)
    # invalid code in the data: DC table with a single 1-bit code, data all ones
    dc1 = bytes([0x00, 1] + [0] * 15 + [0])
    body = write_jpeg(sp3, cf3, qt3, qs3, huff="std", eoi=False)
    info3 = parse_jpeg(body)
    spos = [p for m, p in info3["segs"] if m == 0xDA][0]
    hdr = body[:spos] + seg(0xC4, dc1) + body[spos:spos + 2 + struct.unpack_from(">H", body, spos + 2)[0]]
    add("bad Huffman code", hdr + b"\xFF\x00" * 8 + b"\xFF\xD9", -4)
    # DC table symbol 16+ (bad table) used by the scan
    dcbad = bytes([0x00, 1] + [0] * 15 + [17])
    add("DC symbol > 15", body[:spos] + seg(0xC4, dcbad) + hdr[len(body[:spos]) + len(seg(0xC4, dc1)):] +
        b"\x00\x00\xFF\xD9", -4)
    return recs


# ================================================================ suite: damaged files vs libjpeg

def suite_damaged(rng, ext):
    """Truncated and corrupted files.  Sequential: whenever we return an image it
    must equal libjpeg's (zero-filled data, grey rest of the interval, restart
    resync); an invalid Huffman code gives ERR_CODE (libjpeg warns and goes on).
    Progressive: a partial image of the right size (libjpeg additionally smooths
    incomplete coefficients, so no exact comparison)."""
    seq, prog = [], []
    sp, cf, qt, qs = make_case(rng, 120, 90, SAMPLINGS["420"], "ycc", 85)
    seq.append(("420", write_jpeg(sp, cf, qt, qs)))
    seq.append(("420 restart 2", write_jpeg(sp, cf, qt, qs, restart=2)))
    seq.append(("420 3 scans restart 1", write_jpeg(sp, cf, qt, qs, restart=1, scans=[([c], 0, 63, 0, 0) for c in range(3)])))
    prog.append(("prog 420", write_jpeg(sp, cf, qt, qs, mode="prog")))
    prog.append(("prog 420 restart 3", write_jpeg(sp, cf, qt, qs, mode="prog", restart=3)))
    sp, cf, qt, qs = make_case(rng, 64, 64, [(1, 1)], "gray", 85)
    seq.append(("gray", write_jpeg(sp, cf, qt, qs)))
    sp, cf, qt, qs = make_case(rng, 70, 44, SAMPLINGS["422"], "ycc", 75)
    seq.append(("422 restart 5", write_jpeg(sp, cf, qt, qs, restart=5)))
    items = []                              # (name, data, kind, w, h)
    for name, d in seq + prog:
        info = parse_jpeg(d)
        sos = d.index(b"\xFF\xDA")
        for k in range(16):
            cut = sos + 14 + (len(d) - sos - 14) * k // 16
            items.append((f"{name} cut at {cut}/{len(d)}", d[:cut], "prog" if name.startswith("prog") else "cut",
                          info["W"], info["H"]))
    for n in range(400):
        name, d = rng.choice(seq)
        info = parse_jpeg(d)
        d = bytearray(d)
        sos = d.index(b"\xFF\xDA")
        start = sos + 2 + struct.unpack_from(">H", d, sos + 2)[0]
        rst = [i for i in range(start, len(d) - 1) if d[i] == 0xFF and 0xD0 <= d[i + 1] <= 0xD7]
        if rng.random() < 0.5 or not rst:
            for _ in range(rng.randint(1, 3)):
                i = rng.randrange(start, len(d) - 2)
                b = d[i] ^ (1 << rng.randrange(8))
                if d[i] != 0xFF and d[i - 1] != 0xFF and b != 0xFF:
                    d[i] = b
            kind = "flip"
        else:
            i, a = rng.choice(rst), rng.random()
            if a < 0.4:
                d[i + 1] = 0xD0 + rng.randrange(8)
                kind = "RST renumbered"
            elif a < 0.7:
                del d[i:i + 2]
                kind = "RST deleted"
            else:
                d[i:i] = bytes([0xFF, 0xD0 + rng.randrange(8)])
                kind = "RST inserted"
        items.append((f"{name} {kind} #{n}", bytes(d), kind, info["W"], info["H"]))
    lj = ext.libjpeg([(it[0], it[1]) for it in items])
    recs = []
    for name, data, kind, w, h in items:
        ref = lj_rgb(lj[name], None)

        def chk(res, out, ref=ref, kind=kind, w=w, h=h):
            if res != 0:
                if res == -4 and kind == "flip":
                    return None                 # invalid Huffman code
                if ref is None and res in ERR_NAMES:
                    return None                 # libjpeg rejects it too (e.g. cut / hit inside a header)
                return f"error {res} ({ERR_NAMES.get(res)})"
            ow, oh = struct.unpack_from("<II", out)
            if (ow, oh) != (w, h):
                return f"size {ow}x{oh}"
            if kind == "prog" or ref is None:
                return None
            got, _ = bgra_to_rgb(out, w, h)
            d = np.abs(got - ref)
            return None if d.max() == 0 else f"differs from libjpeg: max {d.max()}, {(d.max(-1) > 0).sum()} px"
        recs.append(Rec(OP_JDEC, 0, data, chk, name))
    return recs


# ================================================================ suite: fuzz

def mutate(rng, d):
    d = bytearray(d)
    k = rng.random()
    if k < 0.25:
        return bytes(d[:rng.randrange(len(d))])
    if k < 0.55:
        for _ in range(rng.randint(1, 8)):
            i = rng.randrange(len(d))
            d[i] ^= 1 << rng.randrange(8)
        return bytes(d)
    if k < 0.65:
        i = rng.randrange(len(d))
        d[i:i] = bytes(rng.randrange(256) for _ in range(rng.randint(1, 40)))
        return bytes(d)
    if k < 0.75:
        i = rng.randrange(len(d))
        del d[i:i + rng.randint(1, 60)]
        return bytes(d)
    if k < 0.85:
        i = rng.randrange(len(d))
        d[i] = 0xFF
        if i + 1 < len(d):
            d[i + 1] = rng.choice([0xC0, 0xC2, 0xC4, 0xD0, 0xD7, 0xD9, 0xDA, 0xDB, 0xDD, 0xE1, 0x00, 0xFF])
        return bytes(d)
    # corrupt a segment length
    segs = [i for i in range(len(d) - 3) if d[i] == 0xFF and d[i + 1] in (0xC0, 0xC2, 0xC4, 0xDA, 0xDB, 0xDD, 0xE1)]
    if segs:
        i = rng.choice(segs)
        struct.pack_into(">H", d, i + 2, rng.choice([0, 1, 2, 3, 0xFFFF, rng.randrange(65536)]))
    return bytes(d)


def plausible(data):
    """skip mutants whose frame is huge (slow, memory hungry, pointless)."""
    for m in (0xC0, 0xC1, 0xC2):
        i = data.find(bytes([0xFF, m]))
        if i >= 0 and i + 9 <= len(data):
            h, w = struct.unpack_from(">HH", data, i + 5)
            if w * h > 3_000_000:
                return False
    return True


def suite_fuzz(rng, count, bases, tally):
    """tally: {result code: count} filled while checking"""
    def chk(res, out):
        tally[res] = tally.get(res, 0) + 1
        return hash_ok(res, out)
    recs = []
    while len(recs) < count:
        name, base = rng.choice(bases)
        m = mutate(rng, base)
        if not plausible(m):
            continue
        recs.append(Rec(OP_JHASH, 0, m, chk, f"fuzz {len(recs)} of {name}"))
    return recs


def bench_image(rng, w, h):
    """detailed photo-like image (fine texture and noise, like a sharp camera image)"""
    img = photo(rng, w, h).astype(np.float32)
    r = np.random.default_rng(99)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    tex = 18 * np.sin(xx * 0.9 + np.sin(yy * 0.05) * 6) * np.sin(yy * 0.7 + np.sin(xx * 0.03) * 5)
    img += tex[..., None] + r.normal(0, 6, (h, w, 1)).astype(np.float32)
    return np.clip(img, 0, 255).astype(np.uint8)


# ================================================================ bench

def run_bench(ext, runners, notes, suite):
    rng = random.Random(4242)
    big = ext.path("bench12mp_v2.jpg")
    if not big.exists():
        img = bench_image(rng, 4000, 3000)
        src = ext.path("bench12mp.ppm")
        src.write_bytes(ppm(img))
        ext.ffmpeg([["-i", rel(src), "-pix_fmt", "yuvj420p", "-q:v", "2", rel(big)]])
        src.unlink()
    data = big.read_bytes()
    img2 = photo(rng, 1152, 648)
    a = np.full((648, 1152, 1), 255, np.uint8)
    surf = struct.pack("<III", 1152, 648, 1) + np.concatenate([img2[..., ::-1], a], -1).tobytes()
    for t, runner in runners.items():
        recs = [Rec(OP_JBDEC, 3, data, None, "dec"), Rec(OP_JBENC, 75 | (10 << 8), surf, None, "enc75"),
                Rec(OP_JBENC, 90 | (10 << 8), surf, None, "enc90"), Rec(OP_JBENC, 95 | (10 << 8), surf, None, "enc95")]
        res = runner.batch(recs, [])
        if any(r is None or r[0] != 0 for r in res):
            suite.fail(f"[{t}] benchmark failed: {[None if r is None else r[0] for r in res]}")
            continue
        w, h, us = struct.unpack_from("<IIQ", res[0][1])
        dec_ms = us / 3 / 1000
        line = f"[{t}] decode {w}x{h} 4:2:0 q2 ({len(data) / 1e6:.1f} MB): {dec_ms:6.1f} ms (target < 250)"
        encs = []
        for (q, r) in zip((75, 90, 95), res[1:]):
            us, ln = struct.unpack_from("<QQ", r[1])
            encs.append((q, us / 10 / 1000, ln))
        line += "; encode 1152x648 " + ", ".join(f"q{q} {ms:5.1f} ms ({ln // 1024} KB)" for q, ms, ln in encs) + " (target < 25)"
        notes.append(line)
        if dec_ms > 250 or any(ms > 25 for _, ms, _ in encs):
            suite.fail(f"[{t}] performance target missed")
        else:
            suite.ok()


# ================================================================ main


# ================================================================ CA package: flags, abbreviated streams,
# encoder options, metadata

OP_JDECFL, OP_JABBREV, OP_JENCEX = 25, 26, 27
OP_DECEX, OP_ENCODE, OP_ENCMETA, OP_METAFULL = 240, 252, 260, 261
JDF_STRAIGHT, JDF_NO_ORIENT, JDF_RGB, JDF_YCC, JDF_CMYK = 1, 2, 4, 8, 16
FMT_PARGB, FMT_ARGB = 1, 3


def _chk(suite, cond, msg):
    if cond:
        suite.ok()
    else:
        suite.fail(msg)


def sf_rgb(out):
    """ct_surface_out record -> (w, h, fmt, rgb (H, W, 3) int64, alpha)."""
    w, h, f = struct.unpack_from("<III", out)
    px = np.frombuffer(out, np.uint8, w * h * 4, 12).reshape(h, w, 4).astype(np.int64)
    return w, h, f & 0xFF, px[..., 2::-1], px[..., 3]


def expect_sf(exp, fmt=FMT_PARGB):
    h, w, _ = exp.shape

    def chk(res, out):
        if res != 0:
            return f"error {res} ({ERR_NAMES.get(res)})"
        ow, oh, f, got, a = sf_rgb(out)
        if (ow, oh) != (w, h):
            return f"size {ow}x{oh}, want {w}x{h}"
        if f != fmt:
            return f"fmt {f} != {fmt}"
        if not np.all(a == 255):
            return "alpha != 255"
        d = np.abs(got - exp)
        if d.max():
            y, x = np.unravel_index(np.argmax(d.max(-1)), d.shape[:2])
            return f"max |d| {d.max()} ({(d.max(-1) > 0).sum()} px) at ({x},{y}) got {got[y, x].tolist()} want {exp[y, x].tolist()}"
        return None
    return chk


def split_abbrev(jpg):
    """full baseline / extended JPEG -> (tables-only stream, abbreviated image stream)."""
    segs = []
    p = 2
    while p < len(jpg):
        m = jpg[p + 1]
        if m == 0xDA:
            segs.append((m, jpg[p:]))
            break
        ln = struct.unpack_from(">H", jpg, p + 2)[0]
        segs.append((m, jpg[p:p + 2 + ln]))
        p += 2 + ln
    tables = b"\xFF\xD8" + b"".join(s for m, s in segs if m in (0xDB, 0xC4)) + b"\xFF\xD9"
    image = b"\xFF\xD8" + b"".join(s for m, s in segs if m not in (0xDB, 0xC4))
    return tables, image


def abbrev_in(tables, image):
    return struct.pack("<I", len(tables)) + tables + image


def suite_flags(rng):
    recs = []
    # orientation: applied by default, kept with JDF_NO_ORIENT / IMGD_NO_ORIENT
    for o in range(1, 9):
        spec, coefs, qt, qs = make_case(rng, 23, 13, SAMPLINGS["420"], "ycc", 80)
        qper = [qt[q] for q in qs]
        data = write_jpeg(spec, coefs, qt, qs, exif=(o, o % 2 == 0))
        exp1 = model_expect(spec, coefs, qper, "ycc", o=1)
        expo = model_expect(spec, coefs, qper, "ycc", o=o)
        recs.append(Rec(OP_JDECFL, 0, data, expect_sf(expo), f"orient {o} applied"))
        recs.append(Rec(OP_JDECFL, JDF_NO_ORIENT | JDF_STRAIGHT, data, expect_sf(exp1, FMT_ARGB), f"orient {o} kept"))

        def chk_decex(res, out, exp=exp1, o=o):
            if res != 0:
                return f"error {res}"
            w, h, f = struct.unpack_from("<III", out)
            meta = out[12:92]
            orient, flags = meta[14], struct.unpack_from("<I", meta, 16)[0]
            if orient != o or flags & 32:
                return f"orient {orient} flags {flags:#x}"
            got = np.frombuffer(out, np.uint8, w * h * 4, len(out) - w * h * 4).reshape(h, w, 4)
            return None if (h, w) == exp.shape[:2] and np.array_equal(got[..., 2::-1], exp) else "pixels"
        recs.append(Rec(OP_DECEX, 0, struct.pack("<IIIIQII", 16, 0, 0, 1, 0, 0, 0) + data, chk_decex,
                        f"img_decode_ex NO_ORIENT {o}"))
    # forced colour spaces
    spec, coefs, qt, qs = make_case(rng, 31, 17, SAMPLINGS["444"], "ycc", 85)
    qper = [qt[q] for q in qs]
    ycc = write_jpeg(spec, coefs, qt, qs)
    recs.append(Rec(OP_JDECFL, JDF_RGB, ycc, expect_sf(model_expect(spec, coefs, qper, "rgb")), "JDF_RGB on JFIF"))
    recs.append(Rec(OP_JDECFL, JDF_YCC, ycc, expect_sf(model_expect(spec, coefs, qper, "ycc")), "JDF_YCC on JFIF"))
    rgbf = write_jpeg(spec, coefs, qt, qs, jfif=False, adobe=0)
    recs.append(Rec(OP_JDECFL, 0, rgbf, expect_sf(model_expect(spec, coefs, qper, "rgb")), "Adobe 0 = RGB"))
    recs.append(Rec(OP_JDECFL, JDF_YCC, rgbf, expect_sf(model_expect(spec, coefs, qper, "ycc")), "JDF_YCC on Adobe 0"))
    spec4, coefs4, qt4, qs4 = make_case(rng, 19, 11, [(1, 1)] * 4, "cmyk", 85)
    qper4 = [qt4[q] for q in qs4]
    cm = write_jpeg(spec4, coefs4, qt4, qs4, jfif=False, adobe=0)
    recs.append(Rec(OP_JDECFL, 0, cm, expect_sf(model_expect(spec4, coefs4, qper4, "cmyk", 0)), "Adobe CMYK inverted"))
    recs.append(Rec(OP_JDECFL, JDF_CMYK, cm, expect_sf(model_expect(spec4, coefs4, qper4, "cmyk", 1)),
                    "JDF_CMYK plain"))
    return recs


def suite_abbrev(rng):
    recs = []
    for name, samp, cs in (("gray", [(1, 1)], "gray"), ("444", SAMPLINGS["444"], "ycc"), ("420", SAMPLINGS["420"], "ycc"),
                           ("422", SAMPLINGS["422"], "ycc"), ("cmyk", [(1, 1)] * 4, "cmyk")):
        for (w, h) in ((1, 1), (16, 8), (37, 21)):
            spec, coefs, qt, qs = make_case(rng, w, h, samp, cs, 75)
            qper = [qt[q] for q in qs]
            for mode, ri in (("baseline", 0), ("ext", 2)):
                full = write_jpeg(spec, coefs, qt, qs, mode=mode, restart=ri, jfif=cs != "cmyk", com=False)
                tables, image = split_abbrev(full)
                exp = model_expect(spec, coefs, qper, cs, 1 if cs == "cmyk" else 0)
                flags = JDF_CMYK if cs == "cmyk" else (JDF_YCC if cs == "ycc" else 0)
                nm = f"{name} {w}x{h} {mode} ri{ri}"
                recs.append(Rec(OP_JABBREV, flags, abbrev_in(tables, image), expect_sf(exp), "abbrev " + nm))
                recs.append(Rec(OP_JABBREV, flags | JDF_STRAIGHT, abbrev_in(b"", full), expect_sf(exp, FMT_ARGB),
                                "no tables " + nm))
                recs.append(Rec(OP_JABBREV, flags, abbrev_in(tables, image[:2] + b"\xFF\xFF" + image[2:]),
                                expect_sf(exp), "fill byte " + nm))
                if cs == "ycc":
                    recs.append(Rec(OP_JABBREV, JDF_RGB, abbrev_in(tables, image),
                                    expect_sf(model_expect(spec, coefs, qper, "rgb")), "abbrev JDF_RGB " + nm))
    # tables in the image stream replace the tables-only stream's (per slot)
    spec, coefs, qt, qs = make_case(rng, 24, 16, SAMPLINGS["420"], "ycc", 60)
    qper = [qt[q] for q in qs]
    full = write_jpeg(spec, coefs, qt, qs)
    tables, image = split_abbrev(full)
    bad_q = {0: [1] * 64, 1: [1] * 64}
    wrong_tables = split_abbrev(write_jpeg(spec, coefs, bad_q, qs))[0]
    recs.append(Rec(OP_JABBREV, JDF_YCC, abbrev_in(wrong_tables, full), expect_sf(model_expect(spec, coefs, qper, "ycc")),
                    "image tables override"))
    # MJPEG style: no DHT anywhere -> Annex K tables
    mj = write_jpeg(spec, coefs, qt, qs, huff="none")
    t2, i2 = split_abbrev(mj)
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(t2, i2), expect_sf(model_expect(spec, coefs, qper, "ycc")), "no DHT"))
    # errors
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(b"", image), expect_err(-2), "no DQT anywhere"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(full, image), expect_err(-2), "SOF in the tables stream"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(tables[2:], image), expect_err(-2), "tables without SOI"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(tables[:1], image), expect_err(-1), "tables 1 byte"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(tables[:-2], image), expect_sf(model_expect(spec, coefs, qper, "ycc")),
                    "tables without EOI"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(tables, b"\xFF\xD8\xFF\xD9"), expect_err(-2), "image EOI only"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(tables, image[:30]), expect_err(-1, -2), "image cut in SOF"))
    recs.append(Rec(OP_JABBREV, 0, abbrev_in(tables, b""), expect_err(-1), "image empty"))
    return recs


def app_segments(data):
    """-> list of (marker, payload) of the segments before SOS."""
    out = []
    p = 2
    while p + 4 <= len(data):
        m = data[p + 1]
        if m == 0xDA:
            break
        ln = struct.unpack_from(">H", data, p + 2)[0]
        out.append((m, data[p + 4:p + 2 + ln]))
        p += 2 + ln
    return out


def suite_encex(rng, ext, runners, suite, notes):
    """jpeg_encode_ex / img_encode options + metadata writing (ops 27, 252, 260) on every target."""
    ph = photo(rng, 97, 61).astype(np.int64)
    bgra = np.concatenate([ph[..., ::-1], np.full((61, 97, 1), 255)], -1).astype(np.uint8)
    alpha = np.where(np.arange(97)[None, :] < 24, 40, 255) * np.ones((61, 1), np.int64)
    st = bgra.copy()
    st[..., 3] = alpha
    pm = st.astype(np.int64)
    pm[..., :3] = (pm[..., :3] * alpha[..., None] + 127) // 255
    pm = pm.astype(np.uint8)
    white = pm[..., 2::-1].astype(np.int64) + (255 - alpha)[..., None]
    samp_want = {444: (1, 1), 422: (2, 1), 420: (2, 2), 400: None}
    recs, info = [], []
    for chroma in (0, 444, 422, 420, 400):
        for q in (50, 89, 90, 95):
            for unit, xd, yd in ((0, 0, 0), (1, 300, 150), (2, 118, 59)):
                cfg = struct.pack("<4I", chroma, unit, xd, yd)
                for fmt, px in ((FMT_ARGB, st), (FMT_PARGB, pm)):
                    recs.append(Rec(OP_JENCEX, q, cfg + struct.pack("<III", 97, 61, fmt) + px.tobytes(), None,
                                    f"encex chroma {chroma} q{q} unit {unit} fmt {fmt}"))
                    info.append((chroma, q, unit, xd, yd, fmt))
    # img_encode with IO_CHROMA (0 = default rule, 444, 422, 420, 400, garbage 123)
    for chroma in (0, 444, 422, 420, 400, 123):
        opt = struct.pack("<12IQQ", 2, 88, 0, 0, 0, 0, 0, chroma, 0, 0, 0, 0, 0, 0)
        recs.append(Rec(OP_ENCODE, 0, opt + struct.pack("<III", 97, 61, FMT_ARGB) + st.tobytes(), None,
                        f"img_encode chroma {chroma}"))
        info.append((chroma if chroma in (444, 422, 420, 400) else 0, 88, 0, 0, 0, FMT_ARGB))
    results = {t: r.batch(recs, []) for t, r in runners.items()}
    tl = list(runners)
    lj_items = []
    for i, (rec, inf) in enumerate(zip(recs, info)):
        chroma, q, unit, xd, yd, fmt = inf
        res = results[tl[0]][i]
        if res is None or res[0] != 0:
            suite.fail(f"{rec.name}: {res and res[0]}")
            continue
        data = bytes(res[1])
        for t in tl[1:]:
            _chk(suite, results[t][i] is not None and bytes(results[t][i][1]) == data,
                        f"{rec.name}: differs on {t}")
        pj = parse_jpeg(data)
        want_c = chroma or (444 if q >= 90 else 420)
        sw = samp_want[want_c]
        if sw is None:
            _chk(suite, len(pj["comps"]) == 1, f"{rec.name}: 4:0:0 must be 1 component")
        else:
            _chk(suite, len(pj["comps"]) == 3 and (pj["comps"][0][1], pj["comps"][0][2]) == sw and
                        all((c[1], c[2]) == (1, 1) for c in pj["comps"][1:]),
                        f"{rec.name}: sampling {[(c[1], c[2]) for c in pj['comps']]}, want {sw}")
        j = pj["jfif"]
        want_d = (1, 96, 96) if unit == 0 else (unit, xd, yd)
        _chk(suite, j is not None and (j[7],) + struct.unpack(">HH", j[8:12]) == want_d,
                    f"{rec.name}: JFIF density {j and (j[7], j[8:12].hex())}")
        if fmt == FMT_ARGB and rec.op == OP_JENCEX:
            # ARGB input must give exactly the file of the premultiplied input
            pmres = results[tl[0]][i + 1]
            _chk(suite, pmres is not None and bytes(pmres[1]) == data, f"{rec.name}: ARGB != PARGB file")
        lj_items.append((i, data))
    # decode: our decoder == libjpeg-turbo exactly, PSNR vs the source over white
    if ext is not None:
        try:
            lj = ext.libjpeg(lj_items)
        except RuntimeError as e:
            lj = None
            notes.append(f"libjpeg reference unavailable: {e}")
        dec = runners[tl[0]].batch([Rec(OP_JDEC, 0, d, None, "") for _, d in lj_items], [])
        worst = 99.0
        for (i, data), dr in zip(lj_items, dec):
            name = recs[i].name
            if dr is None or dr[0] != 0:
                suite.fail(f"{name}: our decode failed")
                continue
            got, _ = bgra_to_rgb(dr[1], 97, 61)
            if lj is not None:
                ref = lj.get(i)
                if ref is None:
                    suite.fail(f"{name}: libjpeg cannot decode")
                else:
                    r = ref[3].astype(np.int64)
                    if ref[2] == 1:
                        r = np.concatenate([r, r, r], -1)
                    _chk(suite, np.array_equal(r, got), f"{name}: libjpeg != ours (max {np.abs(r - got).max()})")
            chroma, q = info[i][0], info[i][1]
            src = white if chroma != 400 else None
            if src is not None:
                p = psnr(got, src)
                worst = min(worst, p) if q >= 89 else worst
                _chk(suite, p >= (24 if q < 89 else 30), f"{name}: PSNR {p:.2f}")
        notes.append(f"worst PSNR at q >= 89: {worst:.2f} dB")


def suite_jmeta(rng, runners, suite):
    """op 260 (encode with IO_META) -> APP1 EXIF (orientation 1) / APP2 ICC / APP1 XMP / density,
    parsed in Python and read back through op 261."""
    import imgtest_common as it
    ph = photo(rng, 40, 30)
    bgra = np.concatenate([ph[..., ::-1], np.full((30, 40, 1), 255)], -1).astype(np.uint8)
    exif = it.exif_blob(orient=6, xres=(72, 1), yres=(72, 1), unit=2, colorspace=1)
    exif1 = it.exif_blob(orient=1, xres=(72, 1), yres=(72, 1), unit=2, colorspace=1)
    xmp = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'>" + b"z" * 500 + b"</x:xmpmeta>"
    iccs = {"small": bytes(range(256)) * 3, "3 segments": bytes((i * 7) & 255 for i in range(140000))}
    huge_exif = it.exif_blob(orient=3) + bytes(70000)
    cases = []
    for iname, icc in iccs.items():
        for flags in (0, 32, 64, 128, 32 | 64 | 128):
            for dpi, unit in ((0, 0), (300 << 16, 1), (300 << 16, 2)):
                opt = struct.pack("<12IQQ", 2, 90, flags, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
                data = (opt + struct.pack("<6I", dpi, dpi, unit, len(icc), len(exif), len(xmp)) + icc + exif + xmp +
                        struct.pack("<III", 40, 30, FMT_ARGB) + bgra.tobytes())
                cases.append((f"meta {iname} flags {flags:#x} unit {unit}", data, flags, icc, dpi, unit))
    opt = struct.pack("<12IQQ", 2, 90, 64 | 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
    bigx = bytes(70000)
    data = (opt + struct.pack("<6I", 0, 0, 0, 0, len(huge_exif), len(bigx)) + huge_exif + bigx +
            struct.pack("<III", 40, 30, FMT_ARGB) + bgra.tobytes())
    cases.append(("oversized EXIF / XMP skipped", data, 0, b"", 0, 0))
    for t, runner in runners.items():
        res = runner.batch([Rec(OP_ENCMETA, 0, d, None, n) for n, d, *_ in cases], [])
        back = []
        for (name, _, flags, icc, dpi, unit), r in zip(cases, res):
            if r is None or r[0] != 0:
                suite.fail(f"[{t}] {name}: encode {r and r[0]}")
                back.append(None)
                continue
            jpg = bytes(r[1])
            segs = app_segments(jpg)
            ex = [p for m, p in segs if m == 0xE1 and p.startswith(b"Exif\0\0")]
            xm = [p for m, p in segs if m == 0xE1 and p.startswith(b"http://ns.adobe.com/xap/1.0/\0")]
            ic = [p for m, p in segs if m == 0xE2 and p.startswith(b"ICC_PROFILE\0")]
            _chk(suite, segs[0][0] == 0xE0, f"[{t}] {name}: APP0 not first")
            _chk(suite, bool(ex) == bool(flags & 64), f"[{t}] {name}: EXIF presence")
            if ex:
                _chk(suite, ex[0][6:] == exif1, f"[{t}] {name}: EXIF content (orientation 1)")
            _chk(suite, bool(xm) == bool(flags & 128), f"[{t}] {name}: XMP presence")
            if xm:
                _chk(suite, xm[0][29:] == xmp, f"[{t}] {name}: XMP content")
            _chk(suite, bool(ic) == bool(flags & 32), f"[{t}] {name}: ICC presence")
            if ic:
                n = len(ic)
                ok = all(p[12] == k + 1 and p[13] == n for k, p in enumerate(ic))
                _chk(suite, ok and b"".join(p[14:] for p in ic) == icc, f"[{t}] {name}: ICC segments")
            j = [p for m, p in segs if m == 0xE0][0]
            want = (1, 96, 96) if unit == 0 else ((1, 300, 300) if unit == 1 else (2, 118, 118))
            _chk(suite, (j[7],) + struct.unpack(">HH", j[8:12]) == want, f"[{t}] {name}: density {j[7:12].hex()}")
            back.append(jpg)
        # read back through the JPEG metadata scanner (op 261)
        from png_check import meta_full
        rr = runner.batch([Rec(OP_METAFULL, 2 | 4 | 8 | 32, j, None, "") for j in back if j is not None], [])
        k = 0
        for (name, _, flags, icc, dpi, unit), j in zip(cases, back):
            if j is None:
                continue
            r = rr[k]
            k += 1
            if r is None or r[0] != 0:
                suite.fail(f"[{t}] {name}: op 261 {r and r[0]}")
                continue
            m = meta_full(r[1])
            _chk(suite, m["icc"] == (icc if flags & 32 else b""), f"[{t}] {name}: ICC read back")
            _chk(suite, m["exif"] == (exif1 if flags & 64 else b""), f"[{t}] {name}: EXIF read back")
            _chk(suite, m["xmp"] == (xmp if flags & 128 else b""), f"[{t}] {name}: XMP read back")
            _chk(suite, m["orient"] == 1, f"[{t}] {name}: orientation {m['orient']}")
            wd = (96 << 16) if unit == 0 else ((300 << 16) if unit == 1 else (118 * 254 * 65536 + 50) // 100)
            _chk(suite, abs(m["dpix"] - wd) <= 1, f"[{t}] {name}: dpi {m['dpix'] / 65536:.3f}")


def suite_jmfuzz(rng, count):
    """mutated JPEGs with APP1 / APP2 / APP14 metadata through op 240 (all blob flags), op 261,
    op 25 and abbreviated pairs through op 26."""
    import imgtest_common as it
    spec, coefs, qt, qs = make_case(rng, 21, 13, SAMPLINGS["420"], "ycc", 70)
    base = write_jpeg(spec, coefs, qt, qs, exif=(6, True))
    icc = bytes(range(256)) * 2
    segs = [it.jpeg_seg(0xE2, b"ICC_PROFILE\0\1\2" + icc[:300]), it.jpeg_seg(0xE2, b"ICC_PROFILE\0\2\2" + icc[300:]),
            it.jpeg_seg(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x/>"), it.jpeg_seg(0xEE, b"Adobe\0d\0\0\0\0\1")]
    bases = [it.jpeg_insert(base, segs), base]
    tables, image = split_abbrev(write_jpeg(spec, coefs, qt, qs, restart=3))
    recs = []
    for i in range(count):
        b = bases[i % 2]
        m = it.mutate(rng, b, keep=2)
        recs.append(Rec(OP_DECEX, 0, struct.pack("<IIIIQII", [1, 14, 16 | 1 | 2][i % 3], 0, 0, 1, 0, 0, 0) + m,
                        None, f"jmfuzz#{i}"))
        if i % 3 == 0:
            recs.append(Rec(OP_METAFULL, 2 | 4 | 8 | 32, m, None, f"jmfuzz#{i} meta"))
        if i % 3 == 1:
            recs.append(Rec(OP_JDECFL, [JDF_NO_ORIENT, JDF_RGB, JDF_CMYK | JDF_STRAIGHT][i % 3], m, None,
                            f"jmfuzz#{i} fl"))
        if i % 2 == 0:
            t2 = it.mutate(rng, tables, keep=2) if i % 4 == 0 else tables
            i2 = it.mutate(rng, image, keep=2) if i % 4 != 0 else image
            recs.append(Rec(OP_JABBREV, JDF_YCC, abbrev_in(t2, i2), None, f"jmfuzz#{i} abbrev"))

    def chk(res, out):
        return None if res == 0 or -15 <= res <= -1 else f"bad result {res}"
    for r in recs:
        r.check = chk
    return recs


def classic_identity(runners, rng, suite):
    """jpeg_encode (op 21) and jpeg_decode (op 20) of this build == the C0 reference binary."""
    ref = ROOT / "build" / "ca" / "ref" / "codectest_c0.exe"
    if "win" not in runners or not ref.exists():
        suite.notes.append("classic identity: reference binary missing, skipped")
        return
    import imgtest_common as it
    rr = it.Runner("win", runners["win"].exe.parent.parent, work_name="jpeg_ref")
    rr.exe = ref
    recs = []
    for (w, h) in ((1, 1), (7, 9), (33, 17), (97, 61)):
        for fmt, al in ((0, ""), (1, "opaque"), (1, "mixed"), (2, "")):
            data, _ = make_surface(rng, w, h, fmt, al)
            for q in (30, 89, 90, 100):
                recs.append(Rec(OP_JENC, q, data, None, f"classic enc {w}x{h} fmt{fmt} {al} q{q}"))
    new = runners["win"].batch(recs, [])
    old = rr.batch(recs)
    for r, a, b in zip(recs, new, old):
        _chk(suite, a is not None and b is not None and a == b, f"output differs: {r.name}")
    dec = [Rec(OP_JDEC, 0, bytes(a[1]), None, "dec " + r.name) for r, a in zip(recs, new) if a and a[0] == 0]
    for r, a, b in zip(dec, runners["win"].batch(dec, []), rr.batch(dec)):
        _chk(suite, a is not None and b is not None and a == b, f"decode differs: {r.name}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build", help="build directory (default: build)")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--only", default="", help="comma list: pydec,ffdec,gdidec,enc,errors,damaged,fuzz,bench,"
                                                "flags,abbrev,encex,jmeta,classic,jmfuzz")
    ap.add_argument("--fuzz", type=int, default=2000)
    ap.add_argument("--no-bench", action="store_true")
    ap.add_argument("--seed", type=int, default=1234)
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        for t in targets:
            subprocess.run(testenv.build_cmd("codectest", t, out), cwd=ROOT, check=True)   # Rust; asm for rwref
    ext = Ext(out / "jpegcheck")
    runners = {t: Runner(t, out) for t in targets}
    t0 = time.time()
    shared = {}          # suite name -> (recs, extra)
    fuzz_bases = []
    all_ok = True
    report = {t: [] for t in targets}

    def per_target(sname, recs, notes=()):
        nonlocal all_ok
        for t, runner in runners.items():
            s = Suite(sname)
            dt = run_suite(runner, s, recs)
            s.notes = list(notes)
            report[t].append((s, dt))
            all_ok &= s.failed == 0

    if want("pydec"):
        cases = []
        ffcheck = suite_pydec(random.Random(a.seed), cases)
        s = Suite("model")
        t1 = time.time()
        recs = pydec_recs(ext, cases, s)
        report[targets[0]].append((s, time.time() - t1))
        all_ok &= s.failed == 0
        per_target("pydec", recs)
        s = Suite("pywriter")
        t1 = time.time()
        suite_writer_ff(ext, ffcheck, s)
        s.notes.append("ffmpeg (jrevdct) planes vs islow model: <= 1 allowed")
        report[targets[0]].append((s, time.time() - t1))
        all_ok &= s.failed == 0
        fuzz_bases += [c[:2] for c in cases if any(k in c[0] for k in ("restart 3", "prog 420 64x48", "Adobe YCCK 420 prog 23",
                                                                   "EXIF orientation 6 II", "ycc 411 101", "prog restart 3",
                                                                   "ycc mixed 101", "16-bit"))]
    if want("ffdec"):
        cases = []
        recs, stats, notes = suite_ffdec(random.Random(a.seed + 1), ext, cases)
        per_target("ffdec", recs, notes)
        if stats:
            worst = max(stats, key=lambda s: s[1])
            report[targets[0]][-1][0].notes.append(
                f"vs ffmpeg RGB (swscale): worst mean |d| {worst[1]:.3f} p99 {worst[2]} max {worst[3]} ({worst[0]})")
        fuzz_bases += [c for c in cases if "97x61" in c[0]][:4]
    if want("gdidec"):
        cases = []
        recs, stats, notes = suite_gdidec(random.Random(a.seed + 2), ext, cases)
        if recs:
            per_target("gdidec", recs, notes)
            if stats:
                worst = max(stats, key=lambda s: s[1])
                report[targets[0]][-1][0].notes.append(
                    f"vs GDI+ decode: worst mean |d| {worst[1]:.3f} p99 {worst[2]} max {worst[3]} ({worst[0]})")
            fuzz_bases += [c for c in cases if "100x37" in c[0]][:2]
    if want("errors"):
        per_target("errors", suite_errors(random.Random(a.seed + 3)))
    if want("damaged"):
        per_target("damaged", suite_damaged(random.Random(a.seed + 6), ext))
    if want("enc"):
        s = Suite("enc")
        t1 = time.time()
        notes = []
        suite_enc(random.Random(a.seed + 4), ext, runners, s, notes)
        s.notes = notes
        report[targets[0]].append((s, time.time() - t1))
        all_ok &= s.failed == 0
    if want("flags"):
        per_target("flags", suite_flags(random.Random(a.seed + 20)))
    if want("abbrev"):
        per_target("abbrev", suite_abbrev(random.Random(a.seed + 21)))
    if want("encex"):
        s = Suite("encex")
        t1 = time.time()
        notes = []
        suite_encex(random.Random(a.seed + 22), ext, runners, s, notes)
        s.notes = notes
        report[targets[0]].append((s, time.time() - t1))
        all_ok &= s.failed == 0
    if want("jmeta"):
        s = Suite("jmeta")
        t1 = time.time()
        suite_jmeta(random.Random(a.seed + 23), runners, s)
        report[targets[0]].append((s, time.time() - t1))
        all_ok &= s.failed == 0
    if want("classic") and "win" in runners:
        s = Suite("classic")
        t1 = time.time()
        classic_identity(runners, random.Random(a.seed + 24), s)
        report["win"].append((s, time.time() - t1))
        all_ok &= s.failed == 0
    if want("jmfuzz") and a.fuzz > 0:
        per_target("jmfuzz", suite_jmfuzz(random.Random(a.seed + 25), a.fuzz // 2))
    if want("fuzz") and a.fuzz > 0:
        if not fuzz_bases:
            cases = []
            suite_pydec(random.Random(a.seed), cases)
            fuzz_bases = [c[:2] for c in cases if c[0].startswith(("prog 420 64", "restart", "ycc 420 64", "Adobe YCCK"))]
        tally = {}
        per_target("fuzz", suite_fuzz(random.Random(a.seed + 5), a.fuzz, fuzz_bases, tally))
        report[targets[0]][-1][0].notes.append(
            f"{len(fuzz_bases)} base files; results (all targets): " +
            ", ".join(f"{ERR_NAMES.get(k, k)} {v}" for k, v in sorted(tally.items(), key=lambda kv: -kv[1])))
    if want("bench") and not a.no_bench:
        s = Suite("bench")
        t1 = time.time()
        notes = []
        run_bench(ext, runners, notes, s)
        s.notes = notes
        report[targets[0]].append((s, time.time() - t1))
        all_ok &= s.failed == 0
    for t in targets:
        print(f"==== {t}")
        for s, dt in report[t]:
            status = "PASS" if s.failed == 0 else "FAIL"
            print(f"[{t}] {status} {s.name:8s} {s.passed:6d} passed {s.failed:5d} failed  ({dt:.1f}s)")
            for n in s.notes:
                print("        " + n)
            for m in s.msgs:
                print("        ! " + m)
    print(f"jpeg_check: {'ALL PASSED' if all_ok else 'FAILURES'} ({time.time() - t0:.0f}s)")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
