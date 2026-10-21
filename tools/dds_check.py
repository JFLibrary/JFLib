#!/usr/bin/env python3
"""Checks of the DDS reader (ASMLIB/src/img/dds.s) and the BC1..BC7 block decoders
(bc_dec.s) through apps/codectest (ops 100..106 in dds_ops.s, op 240 img_decode_ex).

The reference BC decoders below are written from the D3D11 functional spec (BC6H / BC7
bit layouts, partition / anchor tables, unquantize, interpolation, finish_unquantize).
They are verified against an independent implementation -- Mesa's llvmpipe (WSL, EGL
surfaceless + ctypes: glCompressedTexImage2D / glGetTexImage; BC6H read back as halves)
-- in the "mesa" suite, and the assembly decoders are compared with them exactly.

Suites
  bcblock   bc_decode_block for every format: BC7 all 8 modes x every partition x rotation x
            index selection (+ reserved mode), BC6H all 14 modes x partitions, signed and
            unsigned (+ reserved modes), BC1 four- / three-colour mode, BC2, BC3, BC4 / BC5
            unsigned and signed (both interpolation modes, -128) -> exact vs the reference
  bc6half   bc6h_decode_half -> the reference halves (+0 / -0 equal)
  mesa      the Python reference vs Mesa: BC7 and BC6H exact (halves; -0 == +0), BC1..BC5
            within 2 (Mesa bit-replicates the 5:6:5 endpoints and truncates the divisions;
            the spec formulas are evaluated exactly here)
  surface   bc_decode of odd sizes (edge blocks clipped), truncated data, cancel flag
  formats   DDS files of every supported legacy pixel format / FourCC and DX10 format ->
            img_decode_ex (IMGD_STRAIGHT) == reference conversion; IMGMETA (size, bits,
            model, channels, flags, IM_DXGI, IM_TEXFL, frames); premultiplied (DXT2 / DXT4 /
            alpha mode 2) -> FMT_PARGB path
  layout    mip chains (ID_FRAME = level, beyond -> ERR_LIMIT), cube maps (DX10 and legacy,
            partial faces) as the 4 x 3 cross, texture arrays (element 0), volume textures
            (slice 0 of each mip), 1D textures
  errors    truncated headers / data, bad sizes, dimension fields, unsupported formats
  ffmpeg    ffmpeg's DDS decoder on DXT1/3/5, ATI1/ATI2 (ffmpeg: R / G swapped 3Dc), BC4/BC5
            (blue ignored: ffmpeg rebuilds z), A8R8G8B8, X8R8G8B8, A8B8G8R8, R5G6B5,
            A1R5G5B5 (RGB), L8, L16, A8L8, A8 (ffmpeg: grey) files (tolerances: BC +-2,
            5/6-bit expansion +-4, L16 +-1, 8-bit exact)
  wic       WIC (Windows DDS codec) on BC1..BC3 files, tolerance +-2 (Windows only)
  fuzz      mutated DDS files (all kinds) through img_decode_ex -> any result, no crash
  perf      bc_decode / img_decode_ex of a 4096 x 4096 BC7 texture (target < 150 ms)

  python tools/dds_check.py --target both --out build/cd
  python tools/dds_check.py --target win --only bcblock,mesa --no-build
"""

import random
import struct
import subprocess
import sys
import tempfile
from fractions import Fraction
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402

C = it.C
Rec = it.Rec
SUITES = "bcblock,bc6half,mesa,surface,formats,layout,errors,ffmpeg,wic,fuzz,perf"

OP_BCBLOCK, OP_BC6HALF, OP_BCDEC, OP_BCDEC_T, OP_DECEX_T, OP_BCCANCEL, OP_RAWMETA = range(100, 107)
BCF = dict(BC1=1, BC2=2, BC3=3, BC4=4, BC5=5, BC6H=6, BC7=7, BC4S=8, BC5S=9, BC6HS=10)
BLOCK_BYTES = {1: 8, 2: 16, 3: 16, 4: 8, 5: 16, 6: 16, 7: 16, 8: 8, 9: 16, 10: 16}

# ================================================================== reference BC decoders

P2 = [0xCCCC, 0x8888, 0xEEEE, 0xECC8, 0xC880, 0xFEEC, 0xFEC8, 0xEC80,
      0xC800, 0xFFEC, 0xFE80, 0xE800, 0xFFE8, 0xFF00, 0xFFF0, 0xF000,
      0xF710, 0x008E, 0x7100, 0x08CE, 0x008C, 0x7310, 0x3100, 0x8CCE,
      0x088C, 0x3110, 0x6666, 0x366C, 0x17E8, 0x0FF0, 0x718E, 0x399C,
      0xAAAA, 0xF0F0, 0x5A5A, 0x33CC, 0x3C3C, 0x55AA, 0x9696, 0xA55A,
      0x73CE, 0x13C8, 0x324C, 0x3BDC, 0x6996, 0xC33C, 0x9966, 0x0660,
      0x0272, 0x04E4, 0x4E40, 0x2720, 0xC936, 0x936C, 0x39C6, 0x639C,
      0x9336, 0x9CC6, 0x817E, 0xE718, 0xCCF0, 0x0FCC, 0x7744, 0xEE22]
P3 = [[int(c) for c in w] for w in """
0011001102212222 0001001122112221 0000200122112211 0222002200110111
0000000011221122 0011001100220022 0022002211111111 0011001122112211
0000000011112222 0000111111112222 0000111122222222 0012001200120012
0112011201120112 0122012201220122 0011011211221222 0011200122002220
0001001101121122 0111001120012200 0000112211221122 0022002200221111
0111011102220222 0001000122212221 0000001101220122 0000110022102210
0122012200110000 0012001211222222 0110122112210110 0000011012211221
0022110211020022 0110011020022222 0011012201220011 0000200022112221
0000000211221222 0222002200120011 0011001200220222 0120012001200120
0000111122220000 0120120120120120 0120201212010120 0011220011220011
0011112222000011 0101010122222222 0000000021212121 0022112200221122
0022001100220011 0220122102201221 0101222222220101 0000212121212121
0101010101012222 0222011102220111 0002111200021112 0000211221122112
0222011101110222 0002111211120002 0110011001102222 0000000021122112
0110011022222222 0022001100110022 0022112211220022 0000000000002112
0002000100020001 0222122202221222 0101222222222222 0111201122012220""".split()]
A2 = [15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
      15, 2, 8, 2, 2, 8, 8, 15, 2, 8, 2, 2, 8, 8, 2, 2,
      15, 15, 6, 8, 2, 8, 15, 15, 2, 8, 2, 2, 2, 15, 15, 6,
      6, 2, 6, 8, 15, 15, 2, 2, 15, 15, 15, 15, 15, 2, 2, 15]
A3A = [3, 3, 15, 15, 8, 3, 15, 15, 8, 8, 6, 6, 6, 5, 3, 3,
       3, 3, 8, 15, 3, 3, 6, 10, 5, 8, 8, 6, 8, 5, 15, 15,
       8, 15, 3, 5, 6, 10, 8, 15, 15, 3, 15, 5, 15, 15, 15, 15,
       3, 15, 5, 5, 5, 8, 5, 10, 5, 10, 8, 13, 15, 12, 3, 3]
A3B = [15, 8, 8, 3, 15, 15, 3, 8, 15, 15, 15, 15, 15, 15, 15, 8,
       15, 8, 15, 3, 15, 8, 15, 8, 3, 15, 6, 10, 15, 15, 10, 8,
       15, 3, 15, 10, 10, 8, 9, 10, 6, 15, 8, 15, 3, 6, 6, 8,
       15, 3, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 3, 15, 15, 8]
WT = {2: [0, 21, 43, 64], 3: [0, 9, 18, 27, 37, 46, 55, 64],
      4: [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64]}


def subset_of(ns, part, i):
    if ns == 1:
        return 0
    if ns == 2:
        return (P2[part] >> i) & 1
    return P3[part][i]


def anchors(ns, part):
    return [0] if ns == 1 else [0, A2[part]] if ns == 2 else [0, A3A[part], A3B[part]]


class Bits:
    def __init__(self, b):
        self.v, self.p = int.from_bytes(b, "little"), 0

    def get(self, n):
        r = (self.v >> self.p) & ((1 << n) - 1)
        self.p += n
        return r


BC7_MODES = {  # ns, partition bits, rotation bits, isb, colour bits, alpha bits, epb, spb, ib, ib2
    0: (3, 4, 0, 0, 4, 0, 1, 0, 3, 0), 1: (2, 6, 0, 0, 6, 0, 0, 1, 3, 0),
    2: (3, 6, 0, 0, 5, 0, 0, 0, 2, 0), 3: (2, 6, 0, 0, 7, 0, 1, 0, 2, 0),
    4: (1, 0, 2, 1, 5, 6, 0, 0, 2, 3), 5: (1, 0, 2, 0, 7, 8, 0, 0, 2, 2),
    6: (1, 0, 0, 0, 7, 7, 1, 0, 4, 0), 7: (2, 6, 0, 0, 5, 5, 1, 0, 2, 0)}


def bc7_block(blk):
    """16 bytes -> 16 (r, g, b, a)."""
    br = Bits(blk)
    mode = 0
    while mode < 8 and br.get(1) == 0:
        mode += 1
    if mode == 8:
        return [(0, 0, 0, 0)] * 16
    ns, pb, rb, isb, cb, ab, epb, spb, ib, ib2 = BC7_MODES[mode]
    part, rot, sel = br.get(pb), br.get(rb), br.get(isb)
    ne = 2 * ns
    ep = [[0, 0, 0, 255] for _ in range(ne)]
    for c in range(3):
        for e in range(ne):
            ep[e][c] = br.get(cb)
    if ab:
        for e in range(ne):
            ep[e][3] = br.get(ab)
    pbits = [None] * ne
    if epb:
        for e in range(ne):
            pbits[e] = br.get(1)
    if spb:
        for s in range(ns):
            pbits[2 * s] = pbits[2 * s + 1] = br.get(1)
    for e in range(ne):
        for c in range(4 if ab else 3):
            n = cb if c < 3 else ab
            v = ep[e][c]
            if pbits[e] is not None:
                v, n = (v << 1) | pbits[e], n + 1
            v <<= 8 - n
            ep[e][c] = (v | (v >> n)) & 255
    anc = anchors(ns, part)
    idx = [br.get(ib - (i in anc)) for i in range(16)]
    idx2 = [br.get(ib2 - (i == 0)) for i in range(16)] if ib2 else None
    assert br.p == 128
    out = []
    for i in range(16):
        s = subset_of(ns, part, i)
        e0, e1 = ep[2 * s], ep[2 * s + 1]
        if idx2 is None:
            wc = wa = WT[ib][idx[i]]
        elif sel == 0:
            wc, wa = WT[ib][idx[i]], WT[ib2][idx2[i]]
        else:
            wc, wa = WT[ib2][idx2[i]], WT[ib][idx[i]]
        px = [((64 - wc) * e0[c] + wc * e1[c] + 32) >> 6 for c in range(3)]
        px.append(((64 - wa) * e0[3] + wa * e1[3] + 32) >> 6)
        if rot:
            px[rot - 1], px[3] = px[3], px[rot - 1]
        out.append(tuple(px))
    return out


_F = {n: i for i, n in enumerate("rw rx ry rz gw gx gy gz bw bx by bz d".split())}


def _lay(spec):
    out = []
    for tok in spec.split():
        name, rng = tok.split("[")
        rng = rng.rstrip("]")
        if ":" in rng:
            a, b = (int(x) for x in rng.split(":"))
            bits = list(range(b, a + 1)) if a >= b else list(range(b, a - 1, -1))
        else:
            bits = [int(rng)]
        out += [(_F[name], bit) for bit in bits]
    return out


# 5-bit mode value -> (mode 1..14, transformed, endpoint bits, delta bits, layout)
BC6_MODES = {
    0x00: (1, True, 10, (5, 5, 5), _lay(
        "gy[4] by[4] bz[4] rw[9:0] gw[9:0] bw[9:0] rx[4:0] gz[4] gy[3:0] gx[4:0] bz[0] gz[3:0] "
        "bx[4:0] bz[1] by[3:0] ry[4:0] bz[2] rz[4:0] bz[3] d[4:0]")),
    0x01: (2, True, 7, (6, 6, 6), _lay(
        "gy[5] gz[4] gz[5] rw[6:0] bz[0] bz[1] by[4] gw[6:0] by[5] bz[2] gy[4] bw[6:0] bz[3] bz[5] "
        "bz[4] rx[5:0] gy[3:0] gx[5:0] gz[3:0] bx[5:0] by[3:0] ry[5:0] rz[5:0] d[4:0]")),
    0x02: (3, True, 11, (5, 4, 4), _lay(
        "rw[9:0] gw[9:0] bw[9:0] rx[4:0] rw[10] gy[3:0] gx[3:0] gw[10] bz[0] gz[3:0] bx[3:0] bw[10] "
        "bz[1] by[3:0] ry[4:0] bz[2] rz[4:0] bz[3] d[4:0]")),
    0x06: (4, True, 11, (4, 5, 4), _lay(
        "rw[9:0] gw[9:0] bw[9:0] rx[3:0] rw[10] gz[4] gy[3:0] gx[4:0] gw[10] gz[3:0] bx[3:0] bw[10] "
        "bz[1] by[3:0] ry[3:0] bz[0] bz[2] rz[3:0] gy[4] bz[3] d[4:0]")),
    0x0A: (5, True, 11, (4, 4, 5), _lay(
        "rw[9:0] gw[9:0] bw[9:0] rx[3:0] rw[10] by[4] gy[3:0] gx[3:0] gw[10] bz[0] gz[3:0] bx[4:0] "
        "bw[10] by[3:0] ry[3:0] bz[1] bz[2] rz[3:0] bz[4] bz[3] d[4:0]")),
    0x0E: (6, True, 9, (5, 5, 5), _lay(
        "rw[8:0] by[4] gw[8:0] gy[4] bw[8:0] bz[4] rx[4:0] gz[4] gy[3:0] gx[4:0] bz[0] gz[3:0] "
        "bx[4:0] bz[1] by[3:0] ry[4:0] bz[2] rz[4:0] bz[3] d[4:0]")),
    0x12: (7, True, 8, (6, 5, 5), _lay(
        "rw[7:0] gz[4] by[4] gw[7:0] bz[2] gy[4] bw[7:0] bz[3] bz[4] rx[5:0] gy[3:0] gx[4:0] bz[0] "
        "gz[3:0] bx[4:0] bz[1] by[3:0] ry[5:0] rz[5:0] d[4:0]")),
    0x16: (8, True, 8, (5, 6, 5), _lay(
        "rw[7:0] bz[0] by[4] gw[7:0] gy[5] gy[4] bw[7:0] gz[5] bz[4] rx[4:0] gz[4] gy[3:0] gx[5:0] "
        "gz[3:0] bx[4:0] bz[1] by[3:0] ry[4:0] bz[2] rz[4:0] bz[3] d[4:0]")),
    0x1A: (9, True, 8, (5, 5, 6), _lay(
        "rw[7:0] bz[1] by[4] gw[7:0] by[5] gy[4] bw[7:0] bz[5] bz[4] rx[4:0] gz[4] gy[3:0] gx[4:0] "
        "bz[0] gz[3:0] bx[5:0] by[3:0] ry[4:0] bz[2] rz[4:0] bz[3] d[4:0]")),
    0x1E: (10, False, 6, (6, 6, 6), _lay(
        "rw[5:0] gz[4] bz[0] bz[1] by[4] gw[5:0] gy[5] by[5] bz[2] gy[4] bw[5:0] gz[5] bz[3] bz[5] "
        "bz[4] rx[5:0] gy[3:0] gx[5:0] gz[3:0] bx[5:0] by[3:0] ry[5:0] rz[5:0] d[4:0]")),
    0x03: (11, False, 10, (10, 10, 10), _lay("rw[9:0] gw[9:0] bw[9:0] rx[9:0] gx[9:0] bx[9:0]")),
    0x07: (12, True, 11, (9, 9, 9), _lay(
        "rw[9:0] gw[9:0] bw[9:0] rx[8:0] rw[10] gx[8:0] gw[10] bx[8:0] bw[10]")),
    0x0B: (13, True, 12, (8, 8, 8), _lay(
        "rw[9:0] gw[9:0] bw[9:0] rx[7:0] rw[10:11] gx[7:0] gw[10:11] bx[7:0] bw[10:11]")),
    0x0F: (14, True, 16, (4, 4, 4), _lay(
        "rw[9:0] gw[9:0] bw[9:0] rx[3:0] rw[10:15] gx[3:0] gw[10:15] bx[3:0] bw[10:15]")),
}
BC6_RESERVED = (0x13, 0x17, 0x1B, 0x1F)


def _sext(v, n):
    v &= (1 << n) - 1
    return v - (1 << n) if v & (1 << (n - 1)) else v


def _unq(c, bits, signed):
    if signed:
        if bits >= 16:
            return c
        s, c = c < 0, abs(c)
        u = 0 if c == 0 else 0x7FFF if c >= (1 << (bits - 1)) - 1 else ((c << 15) + 0x4000) >> (bits - 1)
        return -u if s else u
    if bits >= 15 or c == 0:
        return c
    return 0xFFFF if c == (1 << bits) - 1 else ((c << 16) + 0x8000) >> bits


def _finish(c, signed):
    if signed:
        m = ((-c) * 31) >> 5 if c < 0 else (c * 31) >> 5
        return (0x8000 | m) if c < 0 and m else m
    return (c * 31) >> 6


def bc6_block_half(blk, signed):
    """16 bytes -> 16 (r, g, b) half bit patterns (reserved modes: 0)."""
    br = Bits(blk)
    mv = br.get(2)
    if mv > 1:
        mv |= br.get(3) << 2
    if mv not in BC6_MODES:
        return [(0, 0, 0)] * 16
    mi, tr, epb, dl, lay = BC6_MODES[mv]
    fld = [0] * 13
    for f, b in lay:
        fld[f] |= br.get(1) << b
    ns = 2 if mi <= 10 else 1
    e = [[fld[4 * c + k] for c in range(3)] for k in range(4)]
    if signed:
        e[0] = [_sext(e[0][c], epb) for c in range(3)]
    if signed or tr:
        for k in range(1, 2 * ns):
            e[k] = [_sext(e[k][c], dl[c] if tr else epb) for c in range(3)]
    if tr:
        for k in range(1, 2 * ns):
            e[k] = [(e[k][c] + e[0][c]) & ((1 << epb) - 1) for c in range(3)]
            if signed:
                e[k] = [_sext(e[k][c], epb) for c in range(3)]
    e = [[_unq(e[k][c], epb, signed) for c in range(3)] for k in range(2 * ns)]
    part = fld[12] if ns == 2 else 0
    ib = 3 if ns == 2 else 4
    anc = anchors(ns, part)
    idx = [br.get(ib - (i in anc)) for i in range(16)]
    assert br.p == 128
    out = []
    for i in range(16):
        s = subset_of(ns, part, i)
        w = WT[ib][idx[i]]
        out.append(tuple(_finish(((64 - w) * e[2 * s][c] + w * e[2 * s + 1][c] + 32) >> 6, signed)
                         for c in range(3)))
    return out


def half_to_u8(h):
    """round(clamp(half, 0, 1) * 255) (ties up); negative / NaN -> 0, +inf -> 255."""
    if h & 0x8000:
        return 0
    if h >= 0x3C00:
        return 255 if h <= 0x7C00 else 0
    ex, m = h >> 10, h & 0x3FF
    mant = m | (1024 if ex else 0)
    s = 25 - max(ex, 1)
    return (255 * mant + (1 << (s - 1))) >> s


def bc6_block(blk, signed):
    return [(half_to_u8(r), half_to_u8(g), half_to_u8(b), 255) for r, g, b in bc6_block_half(blk, signed)]


def _rnd(fr):
    return int((fr * 2 + 1) // 2)


def _rnd_down(fr):
    """round half down (BC1's 1/2 mix: exact ties 127.5 / 42.5 go down, like WIC)"""
    return int(-((-(fr * 2 - 1)) // 2))


def bc1_block(blk, four=False):
    c0, c1 = blk[0] | blk[1] << 8, blk[2] | blk[3] << 8

    def ex(c):
        return (Fraction((c >> 11) & 31, 31), Fraction((c >> 5) & 63, 63), Fraction(c & 31, 31))
    a, b = ex(c0), ex(c1)
    cols = [tuple(_rnd(v * 255) for v in a) + (255,), tuple(_rnd(v * 255) for v in b) + (255,)]
    if four or c0 > c1:
        cols.append(tuple(_rnd((2 * x + y) / 3 * 255) for x, y in zip(a, b)) + (255,))
        cols.append(tuple(_rnd((x + 2 * y) / 3 * 255) for x, y in zip(a, b)) + (255,))
    else:
        cols.append(tuple(_rnd_down((x + y) / 2 * 255) for x, y in zip(a, b)) + (255,))
        cols.append((0, 0, 0, 0))
    idx = int.from_bytes(blk[4:8], "little")
    return [cols[(idx >> (2 * i)) & 3] for i in range(16)]


def bc4_vals(blk, signed):
    r0, r1 = blk[0], blk[1]
    if signed:
        s0, s1 = (r0 - 256 if r0 > 127 else r0), (r1 - 256 if r1 > 127 else r1)
        v0, v1 = Fraction(max(s0, -127), 127), Fraction(max(s1, -127), 127)
        big, lo, hi = s0 > s1, Fraction(-1), Fraction(1)
    else:
        v0, v1 = Fraction(r0, 255), Fraction(r1, 255)
        big, lo, hi = r0 > r1, Fraction(0), Fraction(1)
    vals = [v0, v1]
    if big:
        vals += [((7 - i) * v0 + i * v1) / 7 for i in range(1, 7)]
    else:
        vals += [((5 - i) * v0 + i * v1) / 5 for i in range(1, 5)] + [lo, hi]
    return [_rnd((v + 1) / 2 * 255) if signed else _rnd(v * 255) for v in vals]


def bc4_chan(blk, signed):
    vals = bc4_vals(blk, signed)
    idx = int.from_bytes(blk[2:8], "little")
    return [vals[(idx >> (3 * i)) & 7] for i in range(16)]


def ref_block(fmt, blk):
    """Reference decode of one block -> list of 16 (r, g, b, a)."""
    if fmt == 1:
        return bc1_block(blk)
    if fmt == 2:
        a = int.from_bytes(blk[0:8], "little")
        return [c[:3] + (((a >> (4 * i)) & 15) * 17,) for i, c in enumerate(bc1_block(blk[8:], True))]
    if fmt == 3:
        return [c[:3] + (a,) for c, a in zip(bc1_block(blk[8:], True), bc4_chan(blk[:8], False))]
    if fmt in (4, 8):
        return [(v, v, v, 255) for v in bc4_chan(blk, fmt == 8)]
    if fmt in (5, 9):
        return [(x, y, 0, 255) for x, y in zip(bc4_chan(blk[:8], fmt == 9), bc4_chan(blk[8:], fmt == 9))]
    if fmt in (6, 10):
        return bc6_block(blk, fmt == 10)
    return bc7_block(blk)


def ref_blocks_bgra(fmt, blocks):
    """list of blocks -> bytes, 64 per block (row-major BGRA), like op 100."""
    out = bytearray()
    for b in blocks:
        for r, g, bl, a in ref_block(fmt, b):
            out += bytes((bl, g, r, a))
    return bytes(out)


def ref_image(fmt, data, w, h):
    """Blocks in raster order -> (h, w, 4) BGRA."""
    bw, bh = (w + 3) // 4, (h + 3) // 4
    bs = BLOCK_BYTES[fmt]
    img = np.zeros((bh * 4, bw * 4, 4), np.uint8)
    for by in range(bh):
        for bx in range(bw):
            o = (by * bw + bx) * bs
            px = ref_block(fmt, data[o:o + bs])
            for i, (r, g, b, a) in enumerate(px):
                img[by * 4 + i // 4, bx * 4 + i % 4] = (b, g, r, a)
    return img[:h, :w]


# ================================================================== block generators

def gen_bc7(rng, mode, part=None, rot=None, sel=None):
    v = rng.getrandbits(128)
    if mode == 8:
        return (v & ~0xFF).to_bytes(16, "little")
    v = (v & ~((1 << (mode + 1)) - 1)) | (1 << mode)
    ns, pb, rb, isb = BC7_MODES[mode][:4]
    pos = mode + 1
    for val, n in ((part, pb), (rot, rb), (sel, isb)):
        if n and val is not None:
            v = (v & ~(((1 << n) - 1) << pos)) | (val << pos)
        pos += n
    return v.to_bytes(16, "little")


def gen_bc6(rng, mv, part=None):
    v = rng.getrandbits(128)
    nb = 2 if mv < 2 else 5
    v = (v & ~((1 << nb) - 1)) | mv
    if part is not None and mv in BC6_MODES and BC6_MODES[mv][0] <= 10:
        # d[4:0] is the last 5 header bits (82 - 5 .. 81)
        v = (v & ~(31 << 77)) | (part << 77)
    return v.to_bytes(16, "little")


def gen_blocks(rng, fmt):
    """Coverage set of blocks for one format -> list of bytes."""
    out = []
    if fmt == 7:
        for mode in range(8):
            ns, pb, rb, isb = BC7_MODES[mode][:4]
            for part in range(1 << pb):
                for rot in range(1 << rb):
                    for sel in range(1 << isb):
                        for _ in range(2 if pb else 12):
                            out.append(gen_bc7(rng, mode, part, rot, sel))
        out += [gen_bc7(rng, 8) for _ in range(8)]
        out += [bytes(16), bytes([0xFF] * 16), bytes([0x40] + [0xFF] * 15)]
    elif fmt in (6, 10):
        for mv, info in BC6_MODES.items():
            parts = range(32) if info[0] <= 10 else [None] * 16
            for p in parts:
                for _ in range(3):
                    out.append(gen_bc6(rng, mv, p))
        for mv in BC6_RESERVED:
            out += [gen_bc6(rng, mv) for _ in range(4)]
        out += [bytes(16), bytes([0xFF] * 16)]
    elif fmt == 1:
        for _ in range(600):
            b = bytearray(rng.getrandbits(64).to_bytes(8, "little"))
            out.append(bytes(b))
        for c0, c1 in ((0, 0), (0xFFFF, 0xFFFF), (0x1234, 0x1234), (0, 0xFFFF), (0xFFFF, 0), (0xF81F, 0x07E0)):
            for _ in range(4):
                out.append(struct.pack("<HHI", c0, c1, rng.getrandbits(32)))
        for a in range(32):
            for b in range(0, 32, 3):
                out.append(struct.pack("<HHI", (a << 11) | (a << 6) | b, (b << 11) | (b << 6) | a, rng.getrandbits(32)))
    elif fmt in (4, 8):
        for _ in range(600):
            out.append(rng.getrandbits(64).to_bytes(8, "little"))
        for r0, r1 in ((0, 0), (255, 255), (0x80, 0x81), (0x81, 0x80), (0x80, 0x7F), (0x7F, 0x80), (0, 255), (255, 0),
                       (0x80, 0x80), (0x7F, 0x7F), (0, 0x80), (0x80, 0)):
            for _ in range(3):
                out.append(bytes([r0, r1]) + rng.getrandbits(48).to_bytes(6, "little"))
    else:
        for _ in range(500):
            out.append(rng.getrandbits(8 * BLOCK_BYTES[fmt]).to_bytes(BLOCK_BYTES[fmt], "little"))
        if fmt in (2, 3):
            for c0, c1 in ((0, 0xFFFF), (0x1234, 0x1234)):
                out.append(rng.getrandbits(64).to_bytes(8, "little") + struct.pack("<HHI", c0, c1, rng.getrandbits(32)))
        if fmt in (5, 9):
            for r0, r1 in ((0x80, 0x81), (0x81, 0x80), (0, 255)):
                out.append(bytes([r0, r1]) + rng.getrandbits(48).to_bytes(6, "little") +
                           bytes([r1, r0]) + rng.getrandbits(48).to_bytes(6, "little"))
    return out


# ================================================================== Mesa (llvmpipe) reference

MESA_CODE = r'''
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
glCompressedTexImage2D = gp(b"glCompressedTexImage2D", None, u, i, u, i, i, i, i, p)
glGetTexImage = gp(b"glGetTexImage", None, u, i, u, u, p)
glGetError = gp(b"glGetError", u)
glPixelStorei = gp(b"glPixelStorei", None, u, i)
glPixelStorei(0x0D05, 1)
glPixelStorei(0x0CF5, 1)
data = sys.stdin.buffer.read()
out = bytearray()
pos = 0
while pos < len(data):
    glfmt, rfmt, rtype, obpp, bsz, n = struct.unpack_from("<6I", data, pos)
    pos += 24
    blocks = data[pos:pos + n * bsz]
    pos += n * bsz
    bw = min(n, 1024)
    bh = (n + bw - 1) // bw
    blocks += bytes((bw * bh - n) * bsz)
    tex = ctypes.c_uint()
    glGenTextures(1, ctypes.byref(tex))
    glBindTexture(0x0DE1, tex.value)
    buf = ctypes.create_string_buffer(bytes(blocks), len(blocks))
    glCompressedTexImage2D(0x0DE1, 0, glfmt, bw * 4, bh * 4, 0, len(blocks), buf)
    e1 = glGetError()
    ob = ctypes.create_string_buffer(bw * 4 * bh * 4 * obpp)
    glGetTexImage(0x0DE1, 0, rfmt, rtype, ob)
    e2 = glGetError()
    if e1 or e2:
        sys.stderr.write("GL error %x %x\n" % (e1, e2))
        sys.exit(1)
    raw = ob.raw
    W = bw * 4
    for b in range(n):
        by, bx = divmod(b, bw)
        for y in range(4):
            o = ((by * 4 + y) * W + bx * 4) * obpp
            out += raw[o:o + 4 * obpp]
sys.stdout.buffer.write(bytes(out))
'''

# bcfmt -> (GL internal format, read format, read type, bytes per pixel)
MESA_FMT = {1: (0x83F1, 0x1908, 0x1401, 4), 2: (0x83F2, 0x1908, 0x1401, 4), 3: (0x83F3, 0x1908, 0x1401, 4),
            4: (0x8DBB, 0x1903, 0x1406, 4), 8: (0x8DBC, 0x1903, 0x1406, 4),
            5: (0x8DBD, 0x8227, 0x1406, 8), 9: (0x8DBE, 0x8227, 0x1406, 8),
            6: (0x8E8F, 0x1907, 0x140B, 6), 10: (0x8E8E, 0x1907, 0x140B, 6),
            7: (0x8E8C, 0x1908, 0x1401, 4)}


def mesa_decode(jobs):
    """jobs: list of (bcfmt, [blocks]) -> list of raw outputs (per block 16 pixels) or None."""
    parts = []
    for fmt, blocks in jobs:
        g, f, t, bpp = MESA_FMT[fmt]
        parts.append(struct.pack("<6I", g, f, t, bpp, BLOCK_BYTES[fmt], len(blocks)) + b"".join(blocks))
    try:
        p = it.wsl_python(MESA_CODE, b"".join(parts), timeout=1800)
    except Exception:  # noqa: BLE001
        return None
    if p.returncode != 0:
        return None
    res, pos = [], 0
    for fmt, blocks in jobs:
        n = len(blocks) * 16 * MESA_FMT[fmt][3]
        res.append(p.stdout[pos:pos + n])
        pos += n
    return res


# ================================================================== DDS writer

DDPF_ALPHAPIXELS, DDPF_ALPHA, DDPF_FOURCC, DDPF_RGB = 0x1, 0x2, 0x4, 0x40
DDPF_LUMINANCE, DDPF_BUMPDUDV = 0x20000, 0x80000


def fourcc(s):
    return struct.unpack("<I", s.encode() if isinstance(s, str) else s)[0]


def dds_file(w, h, pf, data, mips=1, depth=0, caps2=0, dx10=None, hsize=124, pfsize=32, flags=0x1007):
    """pf = (flags, fourcc, bitcount, r, g, b, a); dx10 = (dxgi, dim, misc, arraysize, misc2)."""
    hdr = struct.pack("<4sIIIIIII", b"DDS ", hsize, flags | (0x20000 if mips > 1 else 0) | (0x800000 if depth else 0),
                      h, w, 0, depth, mips)
    hdr += bytes(44)
    hdr += struct.pack("<II4sIIIII", pfsize, pf[0], struct.pack("<I", pf[1]), *pf[2:])
    hdr += struct.pack("<IIIII", 0x1000 | (0x400008 if mips > 1 else 0), caps2, 0, 0, 0)
    assert len(hdr) == 128
    if dx10 is not None:
        hdr += struct.pack("<5I", *dx10)
    return hdr + bytes(data)


def pf_fourcc(fcc):
    return (DDPF_FOURCC, fourcc(fcc) if isinstance(fcc, str) else fcc, 0, 0, 0, 0, 0)


def dx10_file(w, h, dxgi, data, mips=1, arr=1, dim=3, misc=0, misc2=0, depth=0):
    return dds_file(w, h, pf_fourcc("DX10"), data, mips=mips, depth=depth, dx10=(dxgi, dim, misc, arr, misc2))


# ================================================================== reference pixel conversion

def unorm(v, bits):
    m = (1 << bits) - 1
    return ((510 * v.astype(np.int64) + m) // (2 * m)).astype(np.uint8)


def snorm(v, bits):
    m = (1 << (bits - 1)) - 1
    v = v.astype(np.int64)
    v = np.where(v >= (1 << (bits - 1)), v - (1 << bits), v)
    v = np.maximum(v, -m)
    return ((510 * (v + m) + 2 * m) // (4 * m)).astype(np.uint8)


def f32_to_u8(x):
    """img_row_f32_to_u8: max(x, 0) (NaN -> 0), min 1, * 255 + 0.5, truncate (float32)."""
    x = np.asarray(x, np.float32)
    x = np.where(np.isnan(x), np.float32(0), x)
    x = np.minimum(np.maximum(x, np.float32(0)), np.float32(1))
    y = x * np.float32(255)
    y = (y + np.float32(0.5)).astype(np.float32)
    return np.floor(y).astype(np.uint8)


def half_bits_to_u8(hb):
    f = np.asarray(hb, np.uint16).view(np.float16).astype(np.float32)
    return f32_to_u8(f)


# channel spec: (name, shift, width, signed, role) role in 'RGBAY'
def conv_int(raw, bpp, chans, n):
    """raw bytes of n pixels (bpp bytes each) -> (n, 4) BGRA."""
    a = np.frombuffer(raw, np.uint8)[:n * bpp].reshape(n, bpp).astype(np.uint64)
    px = np.zeros(n, np.uint64)
    for i in range(bpp):
        px |= a[:, i] << np.uint64(8 * i)
    out = np.zeros((n, 4), np.uint8)
    out[:, 3] = 255
    for sh, wd, sg, role in chans:
        v = (px >> np.uint64(sh)) & np.uint64((1 << wd) - 1)
        b = snorm(v, wd) if sg else unorm(v, wd)
        if role == "Y":
            out[:, 0] = out[:, 1] = out[:, 2] = b
        else:
            out[:, "BGRA".index(role)] = b
    return out


def map_samples(s, nch):
    """(n, nch) u8 samples (R, G, B, A order) -> (n, 4) BGRA."""
    n = s.shape[0]
    out = np.zeros((n, 4), np.uint8)
    out[:, 3] = 255
    if nch == 1:
        out[:, 0] = out[:, 1] = out[:, 2] = s[:, 0]
        return out
    out[:, 2], out[:, 1] = s[:, 0], s[:, 1]
    if nch >= 3:
        out[:, 0] = s[:, 2]
    if nch == 4:
        out[:, 3] = s[:, 3]
    return out


def conv_r11(raw, n):
    v = np.frombuffer(raw, np.uint32)[:n].astype(np.uint32)
    hr = ((v & 0x7FF) << 4).astype(np.uint16)
    hg = (((v >> 11) & 0x7FF) << 4).astype(np.uint16)
    hb = ((v >> 22) << 5).astype(np.uint16)
    s = np.stack([half_bits_to_u8(hr), half_bits_to_u8(hg), half_bits_to_u8(hb)], -1)
    return map_samples(s, 3)


def conv_e5(raw, n):
    v = np.frombuffer(raw, np.uint32)[:n].astype(np.uint32)
    e = (v >> 27).astype(np.int32)
    scale = np.ldexp(np.float32(1), e - 24).astype(np.float32)
    ch = [((v >> sh) & 0x1FF).astype(np.float32) * scale for sh in (0, 9, 18)]
    s = np.stack([f32_to_u8(c) for c in ch], -1)
    return map_samples(s, 3)


class Fmt:
    """A test pixel format: how to write it (legacy pf or dxgi) and how to decode it."""

    def __init__(self, name, kind, bpp, pf=None, dxgi=None, chans=(), nch=0, bcf=0, premul=False,
                 meta=None):
        self.name, self.kind, self.bpp, self.pf, self.dxgi = name, kind, bpp, pf, dxgi
        self.chans, self.nch, self.bcf, self.premul, self.meta = chans, nch, bcf, premul, meta or {}

    def level_bytes(self, w, h):
        if self.kind == "bc":
            return ((w + 3) // 4) * ((h + 3) // 4) * self.bpp
        return w * h * self.bpp

    def random_data(self, rng, w, h):
        if self.kind == "bc":
            bw, bh = (w + 3) // 4, (h + 3) // 4
            fmt = self.bcf
            if fmt == 7:
                return b"".join(gen_bc7(rng, rng.choice([1, 5, 6, 6, 3, 0, 4, 7])) for _ in range(bw * bh))
            if fmt in (6, 10):
                mvs = list(BC6_MODES)
                return b"".join(gen_bc6(rng, rng.choice(mvs)) for _ in range(bw * bh))
            return bytes(rng.getrandbits(8) for _ in range(bw * bh * self.bpp))
        if self.kind in ("f16", "f32"):
            n = w * h * self.nch
            if self.kind == "f16":
                vals = np.array([rng.choice([rng.uniform(-0.2, 1.3), rng.random(), 0.5, 1.0, 0.0, -0.0])
                                 for _ in range(n)], np.float16)
                vals.view(np.uint16)[::17] = 0x7E00          # NaN
                vals.view(np.uint16)[5::23] = 0x7C00         # +inf
                return vals.tobytes()
            vals = np.array([rng.choice([rng.uniform(-0.2, 1.3), rng.random(), 0.5, 1.0])
                             for _ in range(n)], np.float32)
            vals.view(np.uint32)[::19] = 0x7FC00000
            return vals.tobytes()
        return bytes(rng.getrandbits(8) for _ in range(w * h * self.bpp))

    def decode(self, data, w, h):
        """Reference -> (h, w, 4) BGRA straight (premultiplied data stays as stored)."""
        n = w * h
        if self.kind == "bc":
            img = ref_image(self.bcf, data, w, h)
        elif self.kind == "int":
            img = conv_int(data, self.bpp, self.chans, n).reshape(h, w, 4)
        elif self.kind in ("f16", "f32"):
            s = np.frombuffer(data, np.uint16 if self.kind == "f16" else np.float32)[:n * self.nch]
            s = half_bits_to_u8(s) if self.kind == "f16" else f32_to_u8(s)
            img = map_samples(s.reshape(n, self.nch), self.nch).reshape(h, w, 4)
        elif self.kind == "r11":
            img = conv_r11(data, n).reshape(h, w, 4)
        elif self.kind == "e5":
            img = conv_e5(data, n).reshape(h, w, 4)
        else:
            a = np.frombuffer(data, np.uint8)[:n * 4].reshape(h, w, 4).copy()
            if self.kind in ("rgba8", "rgbx8"):
                a = a[..., [2, 1, 0, 3]].copy()
            if self.kind in ("bgrx8", "rgbx8"):
                a[..., 3] = 255
            img = a
        if self.premul:
            img = img.copy()
            for c in range(3):
                img[..., c] = np.minimum(img[..., c], img[..., 3])
        return img

    def file(self, w, h, data, **kw):
        if self.dxgi is not None:
            misc2 = 2 if self.premul else 0
            return dx10_file(w, h, self.dxgi, data, misc2=misc2, **kw)
        return dds_file(w, h, self.pf, data, **kw)


def CH(sh, wd, role, sg=0):
    return (sh, wd, sg, role)


def legacy_rgb(bits, r, g, b, a=0):
    fl = DDPF_RGB | (DDPF_ALPHAPIXELS if a else 0)
    return (fl, 0, bits, r, g, b, a)


def mask_chans(masks, roles, signed=False):
    out = []
    for m, role in zip(masks, roles):
        if m:
            sh = (m & -m).bit_length() - 1
            wd = (m >> sh).bit_length()
            out.append(CH(sh, wd, role, int(signed)))
    return out


def legacy_int(name, bits, masks, fl=DDPF_RGB, roles="RGBA", signed=False, dxgi=0, bitsmeta=None, **meta):
    r, g, b, a = masks
    flags = fl | (DDPF_ALPHAPIXELS if a and fl in (DDPF_RGB, DDPF_LUMINANCE) else 0)
    return Fmt(name, "int", bits // 8, pf=(flags, 0, bits, r, g, b, a),
               chans=mask_chans(masks, roles, signed), meta=dict(dxgi=dxgi, **meta))


def all_formats():
    F = []
    # legacy uncompressed
    F.append(Fmt("A8R8G8B8", "bgra8", 4, pf=legacy_rgb(32, 0xFF0000, 0xFF00, 0xFF, 0xFF000000), meta=dict(dxgi=87, alpha=1)))
    F.append(Fmt("X8R8G8B8", "bgrx8", 4, pf=legacy_rgb(32, 0xFF0000, 0xFF00, 0xFF), meta=dict(dxgi=88, alpha=0)))
    F.append(Fmt("A8B8G8R8", "rgba8", 4, pf=legacy_rgb(32, 0xFF, 0xFF00, 0xFF0000, 0xFF000000), meta=dict(dxgi=28, alpha=1)))
    F.append(Fmt("X8B8G8R8", "rgbx8", 4, pf=legacy_rgb(32, 0xFF, 0xFF00, 0xFF0000), meta=dict(dxgi=0, alpha=0)))
    F.append(legacy_int("R8G8B8", 24, (0xFF0000, 0xFF00, 0xFF, 0)))
    F.append(legacy_int("R5G6B5", 16, (0xF800, 0x7E0, 0x1F, 0)))
    F.append(legacy_int("A1R5G5B5", 16, (0x7C00, 0x3E0, 0x1F, 0x8000), alpha=1))
    F.append(legacy_int("X1R5G5B5", 16, (0x7C00, 0x3E0, 0x1F, 0)))
    F.append(legacy_int("A4R4G4B4", 16, (0xF00, 0xF0, 0xF, 0xF000), alpha=1))
    F.append(legacy_int("X4R4G4B4", 16, (0xF00, 0xF0, 0xF, 0)))
    F.append(legacy_int("R3G3B2", 8, (0xE0, 0x1C, 0x3, 0)))
    F.append(legacy_int("A8R3G3B2", 16, (0xE0, 0x1C, 0x3, 0xFF00), alpha=1))
    F.append(legacy_int("A2B10G10R10", 32, (0x3FF, 0xFFC00, 0x3FF00000, 0xC0000000), alpha=1))
    F.append(legacy_int("A2R10G10B10", 32, (0x3FF00000, 0xFFC00, 0x3FF, 0xC0000000), alpha=1))
    F.append(legacy_int("G16R16", 32, (0xFFFF, 0xFFFF0000, 0, 0)))
    F.append(legacy_int("R11G11B10masks", 32, (0x7FF, 0x3FF800, 0xFFC00000, 0)))
    F.append(legacy_int("R32mask", 32, (0xFFFFFFFF, 0, 0, 0)))
    F.append(legacy_int("L8", 8, (0xFF, 0, 0, 0), fl=DDPF_LUMINANCE, roles="YGBA", gray=1))
    F.append(legacy_int("L16", 16, (0xFFFF, 0, 0, 0), fl=DDPF_LUMINANCE, roles="YGBA", gray=1))
    F.append(legacy_int("A8L8", 16, (0xFF, 0, 0, 0xFF00), fl=DDPF_LUMINANCE, roles="YGBA", gray=1, alpha=1))
    F.append(legacy_int("A4L4", 8, (0xF, 0, 0, 0xF0), fl=DDPF_LUMINANCE, roles="YGBA", gray=1, alpha=1))
    F.append(legacy_int("A8", 8, (0, 0, 0, 0xFF), fl=DDPF_ALPHA, gray=1, alpha=1, dxgi=65))
    F.append(legacy_int("V8U8", 16, (0xFF, 0xFF00, 0, 0), fl=DDPF_BUMPDUDV, signed=True))
    F.append(legacy_int("Q8W8V8U8", 32, (0xFF, 0xFF00, 0xFF0000, 0xFF000000), fl=DDPF_BUMPDUDV, signed=True, alpha=1))
    F.append(legacy_int("V16U16", 32, (0xFFFF, 0xFFFF0000, 0, 0), fl=DDPF_BUMPDUDV, signed=True))
    # legacy FourCC
    for nm, fcc, bcf in (("DXT1", "DXT1", 1), ("DXT3", "DXT3", 2), ("DXT5", "DXT5", 3), ("ATI1", "ATI1", 4),
                         ("BC4U", "BC4U", 4), ("BC4S", "BC4S", 8), ("ATI2", "ATI2", 5), ("BC5U", "BC5U", 5),
                         ("BC5S", "BC5S", 9)):
        F.append(Fmt(nm, "bc", BLOCK_BYTES[bcf], pf=pf_fourcc(fcc), bcf=bcf))
    F.append(Fmt("DXT2", "bc", 16, pf=pf_fourcc("DXT2"), bcf=2, premul=True))
    F.append(Fmt("DXT4", "bc", 16, pf=pf_fourcc("DXT4"), bcf=3, premul=True))
    Y, R4 = "Y", "RGBA"
    F.append(Fmt("D3D36", "int", 8, pf=pf_fourcc(36), chans=[CH(0, 16, "R"), CH(16, 16, "G"), CH(32, 16, "B"), CH(48, 16, "A")]))
    F.append(Fmt("D3D110", "int", 8, pf=pf_fourcc(110), chans=[CH(0, 16, "R", 1), CH(16, 16, "G", 1), CH(32, 16, "B", 1), CH(48, 16, "A", 1)]))
    F.append(Fmt("D3D111", "f16", 2, pf=pf_fourcc(111), nch=1))
    F.append(Fmt("D3D112", "f16", 4, pf=pf_fourcc(112), nch=2))
    F.append(Fmt("D3D113", "f16", 8, pf=pf_fourcc(113), nch=4))
    F.append(Fmt("D3D114", "f32", 4, pf=pf_fourcc(114), nch=1))
    F.append(Fmt("D3D115", "f32", 8, pf=pf_fourcc(115), nch=2))
    F.append(Fmt("D3D116", "f32", 16, pf=pf_fourcc(116), nch=4))
    # DX10
    D = []
    D.append((2, "f32", 16, dict(nch=4)))
    D.append((6, "f32", 12, dict(nch=3)))
    D.append((10, "f16", 8, dict(nch=4)))
    for dx in (9, 11):
        D.append((dx, "int", 8, dict(chans=[CH(0, 16, "R"), CH(16, 16, "G"), CH(32, 16, "B"), CH(48, 16, "A")])))
    D.append((13, "int", 8, dict(chans=[CH(0, 16, "R", 1), CH(16, 16, "G", 1), CH(32, 16, "B", 1), CH(48, 16, "A", 1)])))
    D.append((16, "f32", 8, dict(nch=2)))
    for dx in (23, 24):
        D.append((dx, "int", 4, dict(chans=[CH(0, 10, "R"), CH(10, 10, "G"), CH(20, 10, "B"), CH(30, 2, "A")])))
    D.append((26, "r11", 4, {}))
    for dx in (27, 28, 29):
        D.append((dx, "rgba8", 4, {}))
    D.append((31, "int", 4, dict(chans=[CH(0, 8, "R", 1), CH(8, 8, "G", 1), CH(16, 8, "B", 1), CH(24, 8, "A", 1)])))
    D.append((34, "f16", 4, dict(nch=2)))
    D.append((35, "int", 4, dict(chans=[CH(0, 16, "R"), CH(16, 16, "G")])))
    D.append((37, "int", 4, dict(chans=[CH(0, 16, "R", 1), CH(16, 16, "G", 1)])))
    D.append((41, "f32", 4, dict(nch=1)))
    D.append((49, "int", 2, dict(chans=[CH(0, 8, "R"), CH(8, 8, "G")])))
    D.append((51, "int", 2, dict(chans=[CH(0, 8, "R", 1), CH(8, 8, "G", 1)])))
    D.append((54, "f16", 2, dict(nch=1)))
    D.append((56, "int", 2, dict(chans=[CH(0, 16, Y)])))
    D.append((58, "int", 2, dict(chans=[CH(0, 16, Y, 1)])))
    D.append((61, "int", 1, dict(chans=[CH(0, 8, Y)])))
    D.append((63, "int", 1, dict(chans=[CH(0, 8, Y, 1)])))
    D.append((65, "int", 1, dict(chans=[CH(0, 8, "A")])))
    D.append((67, "e5", 4, {}))
    for dx, bcf in ((70, 1), (71, 1), (72, 1), (73, 2), (74, 2), (75, 2), (76, 3), (77, 3), (78, 3), (79, 4), (80, 4),
                    (81, 8), (82, 5), (83, 5), (84, 9), (94, 6), (95, 6), (96, 10), (97, 7), (98, 7), (99, 7)):
        D.append((dx, "bc", BLOCK_BYTES[bcf], dict(bcf=bcf)))
    D.append((85, "int", 2, dict(chans=[CH(0, 5, "B"), CH(5, 6, "G"), CH(11, 5, "R")])))
    D.append((86, "int", 2, dict(chans=[CH(0, 5, "B"), CH(5, 5, "G"), CH(10, 5, "R"), CH(15, 1, "A")])))
    for dx in (87, 90, 91):
        D.append((dx, "bgra8", 4, {}))
    for dx in (88, 92, 93):
        D.append((dx, "bgrx8", 4, {}))
    D.append((115, "int", 2, dict(chans=[CH(0, 4, "B"), CH(4, 4, "G"), CH(8, 4, "R"), CH(12, 4, "A")])))
    D.append((191, "int", 2, dict(chans=[CH(0, 4, "A"), CH(4, 4, "B"), CH(8, 4, "G"), CH(12, 4, "R")])))
    for dx, kind, bpp, kw in D:
        F.append(Fmt(f"DXGI{dx}", kind, bpp, dxgi=dx, meta=dict(dxgi=dx), **kw))
    F.append(Fmt("DXGI28pm", "rgba8", 4, dxgi=28, premul=True, meta=dict(dxgi=28)))
    F.append(Fmt("DXGI98pm", "bc", 16, dxgi=98, bcf=7, premul=True, meta=dict(dxgi=98)))
    return F


SRGB_DXGI = {29, 72, 75, 78, 91, 93, 99}


# ================================================================== op records

def raw_meta(out):
    """128-byte IMGMETA -> dict of the fields the checks use."""
    b = bytes(out)
    w, h = struct.unpack_from("<II", b, 4)
    return dict(w=w, h=h, bits=b[12], model=b[13], channels=b[15], flags=struct.unpack_from("<I", b, 16)[0],
                frames=struct.unpack_from("<I", b, 20)[0], dxgi=struct.unpack_from("<H", b, 120)[0],
                texfl=b[122], texn=struct.unpack_from("<I", b, 124)[0])


def expect_img(exp, fmt=None):
    """op 240 with IMGD_STRAIGHT: pixels == exp (premultiplied expectations are converted)."""
    return it.expect_decex(exp, fmt=fmt)


def decex(data, frame=0, flags=None):
    return it.decex_input(data, flags=C.IMGD_STRAIGHT if flags is None else flags, frame=frame)


def premul_to_straight(img):
    return it.unpremul(img)


# ================================================================== suites

def suite_bcblock(rng):
    recs = []
    for name, fmt in BCF.items():
        blocks = gen_blocks(rng, fmt)
        for i in range(0, len(blocks), 3000):
            chunk = blocks[i:i + 3000]
            exp = ref_blocks_bgra(fmt, chunk)

            def chk(res, out, exp=exp, chunk=chunk, fmt=fmt):
                if res != 0:
                    return f"error {it.err_name(res)}"
                if bytes(out) == exp:
                    return None
                n = next(k for k in range(len(chunk)) if out[k * 64:k * 64 + 64] != exp[k * 64:k * 64 + 64])
                return (f"block {n} {chunk[n].hex()}: got {bytes(out[n * 64:n * 64 + 16]).hex()} "
                        f"exp {exp[n * 64:n * 64 + 16].hex()}")
            recs.append(Rec(OP_BCBLOCK, fmt, b"".join(chunk), chk, f"{name} blocks {i}"))
    recs.append(Rec(OP_BCBLOCK, 0, bytes(16), it.expect_err(C.ERR_UNSUPPORTED), "bcfmt 0"))
    recs.append(Rec(OP_BCBLOCK, 11, bytes(16), it.expect_err(C.ERR_UNSUPPORTED), "bcfmt 11"))
    return recs


def suite_bc6half(rng):
    recs = []
    for signed in (0, 1):
        blocks = gen_blocks(rng, 10 if signed else 6)
        exp = []
        for b in blocks:
            for r, g, bl in bc6_block_half(b, signed):
                exp.append((r, g, bl, 0x3C00))

        def chk(res, out, exp=exp, blocks=blocks):
            if res != 0:
                return f"error {it.err_name(res)}"
            got = struct.unpack(f"<{len(out) // 2}H", bytes(out))
            for k in range(len(exp)):
                g4 = got[k * 4:k * 4 + 4]
                e4 = exp[k][:4]
                if any((x & 0x7FFF or x) != (y & 0x7FFF or y) and not (x & 0x7FFF == 0 and y & 0x7FFF == 0)
                       for x, y in zip(g4, e4)):
                    return f"block {k // 16} {blocks[k // 16].hex()} pixel {k % 16}: got {g4} exp {e4}"
            return None
        recs.append(Rec(OP_BC6HALF, signed, b"".join(blocks), chk, f"bc6h halves signed={signed}"))
    return recs


def run_mesa(suite, rng):
    """Python reference vs Mesa."""
    jobs = []
    for fmt in (7, 6, 10, 1, 2, 3, 4, 8, 5, 9):
        jobs.append((fmt, gen_blocks(rng, fmt)))
    res = mesa_decode(jobs)
    if res is None:
        suite.notes.append("Mesa (WSL llvmpipe) not available -- skipped")
        return
    for (fmt, blocks), raw in zip(jobs, res):
        name = [k for k, v in BCF.items() if v == fmt][0]
        exact = worst = 0
        for n, b in enumerate(blocks):
            if fmt in (6, 10):
                ref = bc6_block_half(b, fmt == 10)
                got = [struct.unpack_from("<3H", raw, (n * 16 + i) * 6) for i in range(16)]
                ok = all((x == y) or (x & 0x7FFF == 0 and y & 0x7FFF == 0) for p, q in zip(ref, got) for x, y in zip(p, q))
                suite.check(ok, f"mesa {name} block {b.hex()}: ref {ref[:2]} mesa {got[:2]}")
                continue
            ref = ref_block(fmt, b)
            if fmt in (4, 8, 5, 9):
                nc = 1 if fmt in (4, 8) else 2
                vals = struct.unpack_from(f"<{16 * nc}f", raw, n * 16 * 4 * nc)
                got = []
                for i in range(16):
                    v = [vals[i * nc + c] for c in range(nc)]
                    if fmt in (8, 9):
                        u = [int(np.floor((x + 1) / 2 * 255 + 0.5)) for x in v]
                    else:
                        u = [int(np.floor(x * 255 + 0.5)) for x in v]
                    got.append((u[0], u[0], u[0], 255) if nc == 1 else (u[0], u[1], 0, 255))
            else:
                got = [tuple(raw[(n * 16 + i) * 4:(n * 16 + i) * 4 + 4]) for i in range(16)]
            d = max(abs(x - y) for p, q in zip(ref, got) for x, y in zip(p, q))
            worst = max(worst, d)
            exact += d == 0
            if fmt == 7:
                suite.check(d == 0, f"mesa BC7 block {b.hex()}: ref {ref[:2]} mesa {got[:2]}")
            else:
                suite.check(d <= 2, f"mesa {name} block {b.hex()}: |d| {d}")
        if fmt not in (6, 7, 10):
            suite.notes.append(f"{name}: {exact}/{len(blocks)} blocks identical to Mesa, max |d| {worst}")


def suite_surface(rng):
    recs = []
    for fmt in (1, 3, 5, 7, 10):
        for w, h in ((1, 1), (3, 5), (4, 4), (13, 7), (17, 33)):
            bw, bh = (w + 3) // 4, (h + 3) // 4
            blocks = [gen_blocks(rng, fmt)[rng.randrange(40)] for _ in range(bw * bh)]
            data = b"".join(blocks)
            exp = ref_image(fmt, data, w, h)

            def chk(res, out, exp=exp):
                if res != 0:
                    return f"error {it.err_name(res)}"
                sw, sh, f, _, px = it.parse_surface(out)
                if (sw, sh, f) != (exp.shape[1], exp.shape[0], C.FMT_ARGB):
                    return f"surface {sw}x{sh} fmt {f}"
                return None if np.array_equal(px, exp) else f"pixels differ (max {it.max_abs(px, exp)})"
            recs.append(Rec(OP_BCDEC, fmt, struct.pack("<II", w, h) + data, chk, f"bc_decode fmt {fmt} {w}x{h}"))
            recs.append(Rec(OP_BCDEC, fmt, struct.pack("<II", w, h) + data[:-1], it.expect_err(C.ERR_TRUNC),
                            f"bc_decode fmt {fmt} {w}x{h} truncated"))
            recs.append(Rec(OP_BCCANCEL, fmt, struct.pack("<II", w, h) + data, it.expect_err(C.ERR_CANCEL),
                            f"bc_decode_to cancel fmt {fmt} {w}x{h}"))
    recs.append(Rec(OP_BCDEC, 7, struct.pack("<II", 0, 4) + bytes(16), it.expect_err(C.ERR_DIMS), "bc_decode w 0"))
    recs.append(Rec(OP_BCDEC, 12, struct.pack("<II", 4, 4) + bytes(16), it.expect_err(C.ERR_UNSUPPORTED), "bc_decode bad fmt"))
    return recs


def meta_expect(f, w, h, mips=1, cross=False, extra_fl=0, texn=1):
    """(check function on the raw meta) for format f."""
    def chk(m):
        exp_w, exp_h = (4 * w, 3 * h) if cross else (w, h)
        if (m["w"], m["h"]) != (exp_w, exp_h):
            return f"meta size {m['w']}x{m['h']} != {exp_w}x{exp_h}"
        if m["frames"] != mips:
            return f"frames {m['frames']} != {mips}"
        if "dxgi" in f.meta and m["dxgi"] != f.meta["dxgi"]:
            return f"IM_DXGI {m['dxgi']} != {f.meta['dxgi']}"
        if f.dxgi in SRGB_DXGI and not m["flags"] & C.IMF_SRGB:
            return "IMF_SRGB missing"
        if f.premul and not m["flags"] & C.IMF_SRC_PREMUL:
            return "IMF_SRC_PREMUL missing"
        if "alpha" in f.meta and bool(m["flags"] & C.IMF_ALPHA) != bool(f.meta["alpha"]):
            return f"IMF_ALPHA {m['flags']:#x}"
        if f.meta.get("gray") and m["model"] != C.IMM_GRAY:
            return f"model {m['model']}"
        if f.kind in ("f16", "f32") and not m["flags"] & C.IMF_FLOAT:
            return "IMF_FLOAT missing"
        if cross and not m["texfl"] & C.TXF_CROSS:
            return f"TXF_CROSS missing ({m['texfl']:#x})"
        if extra_fl and (m["texfl"] & extra_fl) != extra_fl:
            return f"IM_TEXFL {m['texfl']:#x} lacks {extra_fl:#x}"
        if m["texn"] != texn:
            return f"IM_TEXN {m['texn']} != {texn}"
        return None
    return chk


def rec_meta(data, chk, name, frame=0):
    def c(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        return chk(raw_meta(out))
    return Rec(OP_RAWMETA, frame, data, c, name)


def suite_formats(rng):
    recs = []
    for f in all_formats():
        for w, h in ((1, 1), (5, 3), (8, 8), (19, 6)):
            data = f.random_data(rng, w, h)
            exp = f.decode(data, w, h)
            if f.premul:
                exp = premul_to_straight(exp)
            file = f.file(w, h, data)
            recs.append(Rec(it.OP_DECEX, 0, decex(file), expect_img(exp), f"{f.name} {w}x{h}"))
            # premultiplied default output: the dispatcher's premultiply of the straight result
            if w == 8:
                pexp = f.decode(data, w, h) if f.premul else it.premul(exp)
                recs.append(Rec(it.OP_DECEX, 0, decex(file, flags=0), expect_img(pexp), f"{f.name} {w}x{h} PARGB"))
                recs.append(rec_meta(file, meta_expect(f, w, h), f"{f.name} meta"))
    # bit counts / flags of a few formats
    return recs


def mip_chain(rng, f, w, h, mips, depth=1):
    levels, data = [], b""
    for l in range(mips):
        lw, lh, ld = max(1, w >> l), max(1, h >> l), max(1, depth >> l)
        slices = [f.random_data(rng, lw, lh) for _ in range(ld)]
        levels.append((lw, lh, slices))
        data += b"".join(slices)
    return levels, data


CROSS = {0: (2, 1), 1: (0, 1), 2: (1, 0), 3: (1, 2), 4: (1, 1), 5: (3, 1)}


def cross_image(faces, lw, lh, f):
    img = np.zeros((3 * lh, 4 * lw, 4), np.uint8)
    for k, data in faces.items():
        cx, cy = CROSS[k]
        img[cy * lh:(cy + 1) * lh, cx * lw:(cx + 1) * lw] = f.decode(data, lw, lh)
    return img


def suite_layout(rng):
    recs = []
    fm = {f.name: f for f in all_formats()}
    for fname in ("A8R8G8B8", "DXT5", "DXGI98", "DXGI10", "R5G6B5", "DXGI95"):
        f = fm[fname]
        # mip chains
        for w, h in ((16, 16), (13, 5), (1, 9)):
            mips = max(w, h).bit_length()
            levels, data = mip_chain(rng, f, w, h, mips)
            file = f.file(w, h, data, mips=mips)
            for l, (lw, lh, sl) in enumerate(levels):
                recs.append(Rec(it.OP_DECEX, 0, decex(file, frame=l), expect_img(f.decode(sl[0], lw, lh)),
                                f"{fname} {w}x{h} mip {l}"))
            recs.append(Rec(it.OP_DECEX, 0, decex(file, frame=mips), it.expect_err(C.ERR_LIMIT), f"{fname} mip {mips}"))
            recs.append(rec_meta(file, meta_expect(f, w, h, mips=mips), f"{fname} {w}x{h} mip meta"))
            # declared mip count larger than possible: clamped
            file2 = f.file(w, h, data, mips=mips + 3)
            recs.append(rec_meta(file2, meta_expect(f, w, h, mips=mips), f"{fname} {w}x{h} clamped mips"))
            # missing last level -> that level ERR_TRUNC, earlier ones fine
            lb = f.level_bytes(levels[-1][0], levels[-1][1])
            file3 = f.file(w, h, data[:-lb], mips=mips)
            recs.append(Rec(it.OP_DECEX, 0, decex(file3, frame=mips - 1), it.expect_err(C.ERR_TRUNC), f"{fname} missing last mip"))
            recs.append(Rec(it.OP_DECEX, 0, decex(file3, frame=0), expect_img(f.decode(levels[0][2][0], w, h)),
                            f"{fname} mip 0 with the last mip missing"))
        # cube maps (legacy, DX10), all mips
        w = h = 8
        mips = 4
        chains = [mip_chain(rng, f, w, h, mips) for _ in range(6)]
        data = b"".join(c[1] for c in chains)
        if f.dxgi is not None:
            file = dx10_file(w, h, f.dxgi, data, mips=mips, misc=4, arr=1, misc2=2 if f.premul else 0)
        else:
            file = dds_file(w, h, f.pf, data, mips=mips, caps2=0x200 | 0xFC00)
        for l in range(mips):
            lw = lh = max(1, w >> l)
            exp = cross_image({k: chains[k][0][l][2][0] for k in range(6)}, lw, lh, f)
            recs.append(Rec(it.OP_DECEX, 0, decex(file, frame=l), expect_img(exp), f"{fname} cube mip {l}"))
        recs.append(rec_meta(file, meta_expect(f, w, h, mips=mips, cross=True, extra_fl=C.TXF_CUBE), f"{fname} cube meta"))
        # legacy partial cube (+Y, -Z only) / cube arrays (2 cubes, first used)
        if f.dxgi is None:
            pc = {2: chains[2], 5: chains[5]}
            file = dds_file(w, h, f.pf, pc[2][1] + pc[5][1], mips=mips, caps2=0x200 | 0x1000 | 0x8000)
            exp = cross_image({2: pc[2][0][0][2][0], 5: pc[5][0][0][2][0]}, w, h, f)
            recs.append(Rec(it.OP_DECEX, 0, decex(file), expect_img(exp), f"{fname} partial cube"))
            file = dds_file(w, h, f.pf, pc[2][1], mips=mips, caps2=0x200 | 0x1000 | 0x8000)
            recs.append(Rec(it.OP_DECEX, 0, decex(file), it.expect_err(C.ERR_TRUNC), f"{fname} partial cube truncated"))
        else:
            more = b"".join(mip_chain(rng, f, w, h, mips)[1] for _ in range(6))
            file = dx10_file(w, h, f.dxgi, data + more, mips=mips, misc=4, arr=2, misc2=2 if f.premul else 0)
            exp = cross_image({k: chains[k][0][0][2][0] for k in range(6)}, w, h, f)
            recs.append(Rec(it.OP_DECEX, 0, decex(file), expect_img(exp), f"{fname} cube array"))
            recs.append(rec_meta(file, meta_expect(f, w, h, mips=mips, cross=True, extra_fl=C.TXF_CUBE, texn=2),
                                 f"{fname} cube array meta"))
            # texture array: element 0 of 3
            arr = [mip_chain(rng, f, 12, 7, 2) for _ in range(3)]
            file = dx10_file(12, 7, f.dxgi, b"".join(a[1] for a in arr), mips=2, arr=3)
            for l in range(2):
                lw, lh, sl = arr[0][0][l]
                recs.append(Rec(it.OP_DECEX, 0, decex(file, frame=l), expect_img(f.decode(sl[0], lw, lh)),
                                f"{fname} array mip {l}"))
            recs.append(rec_meta(file, meta_expect(f, 12, 7, mips=2, extra_fl=C.TXF_ARRAY, texn=3), f"{fname} array meta"))
            # 1D
            d1 = f.random_data(rng, 9, 1)
            file = dx10_file(9, 1, f.dxgi, d1, dim=2)
            recs.append(Rec(it.OP_DECEX, 0, decex(file), expect_img(f.decode(d1, 9, 1)), f"{fname} 1D"))
            recs.append(Rec(it.OP_DECEX, 0, decex(dx10_file(9, 2, f.dxgi, d1 * 2, dim=2)), it.expect_err(C.ERR_HDR), f"{fname} 1D h 2"))
        # volume: depth 4, mips 3
        levels, data = mip_chain(rng, f, 8, 6, 3, depth=4)
        if f.dxgi is not None:
            file = dx10_file(8, 6, f.dxgi, data, mips=3, dim=4, depth=4, misc2=2 if f.premul else 0)
        else:
            file = dds_file(8, 6, f.pf, data, mips=3, depth=4, caps2=0x200000)
        for l, (lw, lh, sl) in enumerate(levels):
            recs.append(Rec(it.OP_DECEX, 0, decex(file, frame=l), expect_img(f.decode(sl[0], lw, lh)),
                            f"{fname} volume mip {l}"))
        recs.append(rec_meta(file, meta_expect(f, 8, 6, mips=3, extra_fl=C.TXF_VOLUME, texn=4), f"{fname} volume meta"))
    return recs


def suite_errors(rng):
    recs = []
    f = [x for x in all_formats() if x.name == "DXT1"][0]
    data = f.random_data(rng, 8, 8)
    good = f.file(8, 8, data)
    E = it.expect_err
    recs.append(Rec(it.OP_DECEX, 0, decex(good), expect_img(f.decode(data, 8, 8)), "good"))
    for n in (0, 1, 3, 4, 60, 127):
        recs.append(Rec(it.OP_DECEX, 0, decex(good[:n]), E(C.ERR_TRUNC, C.ERR_UNSUPPORTED), f"header cut at {n}"))
    for n in (128, 129, 150, len(good) - 1):
        recs.append(Rec(it.OP_DECEX, 0, decex(good[:n]), E(C.ERR_TRUNC), f"data cut at {n}"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(8, 8, data, hsize=100)), E(C.ERR_HDR), "header size 100"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(8, 8, data, pfsize=24)), E(C.ERR_HDR), "pf size 24"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(0, 8, data)), E(C.ERR_DIMS), "w 0"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(8, 0, data)), E(C.ERR_DIMS), "h 0"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(65536, 8, data)), E(C.ERR_DIMS), "w 65536"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(0xFFFFFFFF, 0xFFFFFFFF, data)), E(C.ERR_DIMS), "w h 2^32-1"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(f.file(16384, 16384, data), C.IMGD_STRAIGHT, maxpx=1 << 20),
                    E(C.ERR_LIMIT), "maxpx"))
    recs.append(Rec(it.OP_DECEX, 0, decex(f.file(65535, 65535, data)), E(C.ERR_LIMIT), "65535^2"))
    # huge declared sizes with tiny data fail before the surface is allocated
    recs.append(Rec(it.OP_DECEX, 0, decex(dx10_file(16384, 16384, 98, bytes(64))), E(C.ERR_TRUNC), "16384^2 BC7, 64 bytes"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(16384, 16384, (DDPF_RGB, 0, 32, 0xFF0000, 0xFF00, 0xFF, 0), bytes(64))),
                    E(C.ERR_TRUNC), "16384^2 X8R8G8B8, 64 bytes"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8192, 8192, f.pf, bytes(64), caps2=0xFE00)), E(C.ERR_LIMIT), "8192 cube cross"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(good, C.IMGD_STRAIGHT, cancel=1), E(C.ERR_CANCEL), "cancel"))
    d10 = [x for x in all_formats() if x.name == "DXGI98"][0]
    bd = d10.random_data(rng, 8, 8)
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d10.file(8, 8, bd), C.IMGD_STRAIGHT, cancel=1), E(C.ERR_CANCEL), "cancel BC7"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, pf_fourcc("DX10"), bd, dx10=(98, 3, 0, 0, 0))), E(C.ERR_HDR), "array size 0"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, pf_fourcc("DX10"), bd, dx10=(98, 5, 0, 1, 0))), E(C.ERR_HDR), "dimension 5"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, pf_fourcc("DX10"), bd, dx10=(98, 4, 0, 2, 0))), E(C.ERR_HDR), "3D array"))
    recs.append(Rec(it.OP_DECEX, 0, decex(good[:128] + struct.pack("<5I", 98, 3, 0, 1, 0)), E(C.ERR_TRUNC), "DX10 data cut"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dx10_file(8, 8, 98, bd)[:140]), E(C.ERR_TRUNC), "DX10 header cut"))
    for dx in (0, 1, 3, 12, 20, 45, 100, 103, 107, 113, 130, 190, 192, 1000, 0x10000 + 98):
        recs.append(Rec(it.OP_DECEX, 0, decex(dx10_file(8, 8, dx, bytes(1024))), E(C.ERR_UNSUPPORTED), f"DXGI {dx}"))
    for fcc in ("YUY2", "UYVY", "RGBG", "GRGB", "MET1", "ABCD", 117, 0):
        recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, pf_fourcc(fcc), bytes(1024))), E(C.ERR_UNSUPPORTED), f"FourCC {fcc}"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, (0x200, 0, 16, 0, 0, 0, 0), bytes(1024))), E(C.ERR_UNSUPPORTED), "DDPF_YUV"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, (DDPF_RGB, 0, 16, 0xF0F0, 0x0F00, 0x000F, 0), bytes(1024))),
                    E(C.ERR_UNSUPPORTED), "non-contiguous mask"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, (DDPF_RGB, 0, 12, 0xF00, 0xF0, 0xF, 0), bytes(1024))),
                    E(C.ERR_UNSUPPORTED), "12-bit pixels"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, (DDPF_RGB, 0, 32, 0, 0, 0, 0), bytes(1024))),
                    E(C.ERR_UNSUPPORTED), "no masks"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, (DDPF_BUMPDUDV, 0, 16, 0x1, 0xFFFE, 0, 0), bytes(1024))),
                    E(C.ERR_UNSUPPORTED), "1-bit signed"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, f.pf, data, caps2=0x200)), E(C.ERR_HDR), "cube without faces"))
    recs.append(Rec(it.OP_DECEX, 0, decex(dds_file(8, 8, f.pf, data, depth=70000, caps2=0x200000)), E(C.ERR_HDR), "depth 70000"))
    # probe of an unsupported format still fills the size
    def chk_unsup(res, out):
        m = raw_meta(out)
        return None if res == C.ERR_UNSUPPORTED and (m["w"], m["h"]) == (8, 8) else f"{it.err_name(res)} {m}"
    recs.append(Rec(OP_RAWMETA, 0, dds_file(8, 8, pf_fourcc("YUY2"), bytes(1024)), chk_unsup, "probe YUY2"))
    return recs


# ------------------------------------------------------------------ external references

FFMPEG_CASES = [("DXT1", 2), ("DXT3", 2), ("DXT5", 2), ("ATI1", 2), ("BC4U", 2), ("BC5U", 2), ("ATI2", 2),
                ("A8R8G8B8", 0), ("X8R8G8B8", 0), ("A8B8G8R8", 0), ("R5G6B5", 4), ("A1R5G5B5", 4),
                ("L8", 0), ("L16", 1), ("A8L8", 0), ("A8", 0)]


def run_ffmpeg(suite, runner, rng):
    if not it.have_exe("ffmpeg"):
        suite.notes.append("ffmpeg not found -- skipped")
        return
    fm = {f.name: f for f in all_formats()}
    work = Path(tempfile.mkdtemp(prefix="ddsff_", dir=runner.work))
    recs, notes = [], []
    for name, tol in FFMPEG_CASES:
        f = fm[name]
        w, h = 36, 20
        data = f.random_data(rng, w, h)
        if f.kind == "bc" and f.bcf == 1:
            data = bytes(rng.getrandbits(8) for _ in range(len(data)))
        file = f.file(w, h, data)
        p = work / f"{name}.dds"
        p.write_bytes(file)
        ref = it.ffmpeg_decode(p)
        if ref is None:
            notes.append(f"{name}: ffmpeg cannot decode")
            continue
        chans = [0, 1, 2, 3]
        if f.kind == "bc" and f.bcf == 5:
            chans = [1, 2]                     # ffmpeg reconstructs blue (normal map z)
            if name == "ATI2":
                ref = ref[..., [0, 2, 1, 3]]    # ffmpeg treats ATI2 as 3Dc (R / G swapped)
        if name == "A8":
            ref = ref.copy()
            ref[..., 3] = ref[..., 0]           # ffmpeg shows A8 as grey
            chans = [3]
        if name == "A1R5G5B5":
            chans = [0, 1, 2]                   # ffmpeg ignores the alpha bit (rgb555)

        def chk(res, out, ref=ref, tol=tol, chans=chans, name=name):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            m = it.max_abs(d["px"][..., chans], ref[..., chans])
            notes.append(f"{name}: max |d| vs ffmpeg {m}")
            return None if m <= tol else f"max |d| {m} > {tol}"
        recs.append(Rec(it.OP_DECEX, 0, decex(file), chk, f"ffmpeg {name}"))
    it.run_suite(runner, suite, recs)
    suite.notes += notes


def run_wic(suite, runner, rng):
    if runner.target != "win":
        return
    fm = {f.name: f for f in all_formats()}
    work = Path(tempfile.mkdtemp(prefix="ddswic_", dir=runner.work))
    wic = it.Wic(work)
    jobs = []
    for name in ("DXT1", "DXT3", "DXT5", "DXGI71", "DXGI74", "DXGI77", "A8R8G8B8"):
        f = fm[name]
        for w, h in ((32, 32), (20, 12)):
            data = f.random_data(rng, w, h)
            file = f.file(w, h, data)
            p = work / f"{name}_{w}x{h}.dds"
            p.write_bytes(file)
            jobs.append((name, file, f.decode(data, w, h), wic.decode(p)))
    wic.run()
    recs, notes = [], []
    for name, file, exp, ji in jobs:
        r = wic.result(ji)
        if r is None:
            notes.append(f"{name}: WIC cannot decode")
            continue
        ref = r[3]
        # WIC returns straight BGRA; transparent pixels may carry any colour
        vis = ref[..., 3] > 0

        def chk(res, out, ref=ref, vis=vis, name=name):
            if res != 0:
                return f"error {it.err_name(res)}"
            px = it.parse_decex(out)["px"]
            if not np.array_equal(px[..., 3], ref[..., 3]):
                return "alpha differs from WIC"
            m = it.max_abs(px[vis][:, :3], ref[vis][:, :3]) if vis.any() else 0
            eq = float(np.mean(np.all(px[vis][:, :3] == ref[vis][:, :3], -1))) if vis.any() else 1.0
            notes.append(f"{name}: max |d| vs WIC {m}, {eq * 100:.1f} % identical")
            return None if m <= 2 else f"max |d| {m} > 2"
        recs.append(Rec(it.OP_DECEX, 0, decex(file), chk, f"wic {name}"))
    it.run_suite(runner, suite, recs)
    suite.notes += sorted(set(notes))


def suite_fuzz(rng, n):
    recs = []
    corpus = []
    for f in all_formats():
        w, h = rng.choice([(4, 4), (5, 3), (9, 9)])
        data = f.random_data(rng, w, h)
        corpus.append(f.file(w, h, data, mips=2) if rng.random() < 0.3 else f.file(w, h, data))
    fm = {f.name: f for f in all_formats()}
    f = fm["DXGI98"]
    corpus.append(dx10_file(8, 8, 98, f.random_data(rng, 8, 8) * 7, misc=4, mips=2))
    corpus.append(dds_file(8, 8, fm["DXT1"].pf, bytes(32 * 6 * 2), mips=2, caps2=0xFE00))
    corpus.append(dds_file(8, 8, fm["A8R8G8B8"].pf, bytes(256 * 5), mips=2, depth=4, caps2=0x200000))
    for i in range(n):
        base = rng.choice(corpus)
        m = it.mutate(rng, base, keep=rng.choice([0, 4, 4, 128]))
        flags = rng.choice([0, C.IMGD_STRAIGHT, C.IMGD_PROBE])
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(m, flags, frame=rng.choice([0, 0, 0, 1, 2])), it.any_result, f"fuzz #{i}"))
    for i in range(n // 10):
        fmt = rng.randrange(1, 11)
        blocks = bytes(rng.getrandbits(8) for _ in range(16 * rng.randrange(1, 20)))
        blocks = blocks[:len(blocks) - len(blocks) % BLOCK_BYTES[fmt]]
        recs.append(Rec(OP_BCBLOCK, fmt, blocks, it.any_result, f"fuzz blocks #{i}"))
    return recs


def time_min(runner, op, arg, data, runs=5):
    """Minimum of `runs` timed calls (codectest ops 103 / 104) in one batch -> us or error."""
    recs = [Rec(op, arg, data, None, f"t{i}") for i in range(runs)]
    res = runner.batch(recs)
    best = None
    for r in res:
        if r is None or r[0] != 0:
            return None, (r[0] if r else "crash")
        us = struct.unpack_from("<I", r[1], 0)[0]
        best = us if best is None else min(best, us)
    return best, 0


def run_perf(suite, runner, rng, size=4096):
    nb = (size // 4) ** 2
    mixes = {"random modes": None, "mode 6": [6], "modes 1/5/6": [1, 5, 6], "mode 0": [0]}
    for label, modes in mixes.items():
        if modes is None:
            data = bytes(np.random.default_rng(rng.randrange(1 << 30)).integers(0, 256, nb * 16, np.uint8))
        else:
            one = [gen_bc7(rng, m) for m in modes for _ in range(64)]
            data = b"".join(one[rng.randrange(len(one))] for _ in range(nb))
        us, err = time_min(runner, OP_BCDEC_T, 7 | (1 << 8), struct.pack("<II", size, size) + data)
        if us is None:
            suite.fail(f"perf {label}: {err}")
            continue
        suite.notes.append(f"BC7 {size}x{size} bc_decode ({label}): {us / 1000:.1f} ms (min of 5)")
        suite.check(us < 150000, f"BC7 {label} decode {us / 1000:.1f} ms >= 150 ms")
        if label == "modes 1/5/6":
            us, err = time_min(runner, OP_DECEX_T, 1, dx10_file(size, size, 98, data))
            if us is not None:
                suite.notes.append(f"BC7 {size}x{size} DDS img_decode_ex straight: {us / 1000:.1f} ms (min of 5)")
                suite.check(us < 150000, f"BC7 DDS img_decode_ex {us / 1000:.1f} ms >= 150 ms")
    for fmt, label in ((1, "BC1"), (3, "BC3"), (5, "BC5"), (6, "BC6H")):
        data = bytes(np.random.default_rng(rng.randrange(1 << 30)).integers(0, 256, nb * BLOCK_BYTES[fmt], np.uint8))
        us, err = time_min(runner, OP_BCDEC_T, fmt | (1 << 8), struct.pack("<II", size, size) + data, runs=3)
        if us is not None:
            suite.notes.append(f"{label} {size}x{size} bc_decode: {us / 1000:.1f} ms (min of 3)")


# ================================================================== main

def main():
    ap = it.argparser(__doc__, SUITES)
    ap.add_argument("--perf-size", type=int, default=4096)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        it.build_codectest(targets, out)
    suites = []
    if want("mesa") and not a.no_ext:
        s = it.Suite("mesa", "ref")
        run_mesa(s, random.Random(a.seed + 50))
        suites.append(s)
    for target in targets:
        print(f"==== {target}")
        runner = it.Runner(target, out, work_name="dds")

        def run(name, fn):
            s = it.Suite(name, target)
            it.run_suite(runner, s, fn())
            suites.append(s)
            return s
        if want("bcblock"):
            run("bcblock", lambda: suite_bcblock(random.Random(a.seed)))
        if want("bc6half"):
            run("bc6half", lambda: suite_bc6half(random.Random(a.seed + 1)))
        if want("surface"):
            run("surface", lambda: suite_surface(random.Random(a.seed + 2)))
        if want("formats"):
            run("formats", lambda: suite_formats(random.Random(a.seed + 3)))
        if want("layout"):
            run("layout", lambda: suite_layout(random.Random(a.seed + 4)))
        if want("errors"):
            run("errors", lambda: suite_errors(random.Random(a.seed + 5)))
        if want("ffmpeg") and not a.no_ext:
            s = it.Suite("ffmpeg", target)
            run_ffmpeg(s, runner, random.Random(a.seed + 6))
            suites.append(s)
        if want("wic") and not a.no_ext and target == "win":
            s = it.Suite("wic", target)
            run_wic(s, runner, random.Random(a.seed + 7))
            suites.append(s)
        if want("fuzz"):
            run("fuzz", lambda: suite_fuzz(random.Random(a.seed + 8), a.fuzz))
        if want("perf"):
            s = it.Suite("perf", target)
            run_perf(s, runner, random.Random(a.seed + 9), a.perf_size)
            suites.append(s)
    ok = it.report(suites, "dds_check")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
