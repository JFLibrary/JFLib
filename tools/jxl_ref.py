#!/usr/bin/env python3
"""JPEG XL decoder model (ISO/IEC 18181-1), written from libjxl 0.11.1's decoder sources
(build/ext/libjxl-0.11.1/lib/jxl).  Reference model for the assembly JPEG XL decoder of ASMLIB.

    python tools/jxl_ref.py FILE.jxl [--info] [--out OUT.png]

Parts: bit reader and field coding (fields.cc), size / image metadata / colour encoding headers,
entropy decoding (dec_ans.cc: ANS with alias tables, prefix codes, context maps with MTF, hybrid
uints, LZ77), ICC profile decompression (icc_codec.cc), frame header, loop filter parameters,
TOC with permutation, modular image decoding (MA trees, weighted predictor, RCT / palette /
squeeze), VarDCT (quantization tables, coefficient orders, AC coefficients, the 27 transforms,
adaptive DC smoothing) and the render pipeline (chroma upsampling, gaborish, EPF, upsampling,
YCbCr / XYB, sRGB transfer function).
"""

import math
import sys
from pathlib import Path

import numpy as np


class JXLError(Exception):
    pass


# ====================================================================== bit reader / fields

class Bits:
    """LSB-first bit reader; reads past the end give zero bits (and set overrun)"""

    def __init__(self, data, pos=0, end=None):
        self.d = data
        self.p = pos * 8
        self.end = len(data) if end is None else end
        self.overrun = False

    def f(self, n):
        v = 0
        for i in range(n):
            byte = self.p >> 3
            if byte < self.end:
                b = (self.d[byte] >> (self.p & 7)) & 1
            else:
                b = 0
                self.overrun = True
            v |= b << i
            self.p += 1
        return v

    def peek(self, n):
        p, o = self.p, self.overrun
        v = self.f(n)
        self.p, self.overrun = p, o
        return v

    def u32(self, d0, d1, d2, d3):
        """U32 with four distributions: ('v', value) or ('b', bits, offset)"""
        d = (d0, d1, d2, d3)[self.f(2)]
        if d[0] == 'v':
            return d[1]
        return self.f(d[1]) + d[2]

    def u64(self):
        sel = self.f(2)
        if sel == 0:
            return 0
        if sel == 1:
            return 1 + self.f(4)
        if sel == 2:
            return 17 + self.f(8)
        v = self.f(12)
        shift = 12
        while self.f(1):
            if shift == 60:
                v |= self.f(4) << shift
                break
            v |= self.f(8) << shift
            shift += 8
        return v

    def f16(self):
        b = self.f(16)
        sign, e, m = b >> 15, (b >> 10) & 31, b & 1023
        if e == 31:
            raise JXLError("F16 inf / NaN")
        if e == 0:
            v = m / 1024.0 / 16384.0
        else:
            v = (1.0 + m / 1024.0) * 2.0 ** (e - 15)
        return -v if sign else v

    def boolean(self):
        return self.f(1) == 1

    def enum(self):
        return self.u32(V(0), V(1), B(4, 2), B(6, 18))

    def byte_align(self):
        r = self.p & 7
        if r:
            if self.f(8 - r):
                raise JXLError("nonzero padding bits")

    def extensions(self):
        ext = self.u64()
        if ext:
            total = 0
            for i in range(64):
                if ext >> i & 1:
                    total += self.u64()
            self.p += total
        return ext


def V(v):
    return ('v', v)


def B(bits, offset=0):
    return ('b', bits, offset)


def unpack_signed(u):
    return (u >> 1) ^ -(u & 1)


# ====================================================================== headers

ASPECT = [None, (1, 1), (12, 10), (4, 3), (3, 2), (16, 9), (5, 4), (2, 1)]


class SizeHeader:
    def read(self, b, preview=False):
        if not preview:
            small = b.boolean()
            if small:
                self.ysize = (b.f(5) + 1) * 8
            else:
                self.ysize = b.u32(B(9, 1), B(13, 1), B(18, 1), B(30, 1))
            ratio = b.f(3)
            if ratio:
                n, d = ASPECT[ratio]
                self.xsize = self.ysize * n // d
            elif small:
                self.xsize = (b.f(5) + 1) * 8
            else:
                self.xsize = b.u32(B(9, 1), B(13, 1), B(18, 1), B(30, 1))
        else:
            div8 = b.boolean()
            if div8:
                self.ysize = b.u32(V(16), V(32), B(5, 1), B(9, 33)) * 8
            else:
                self.ysize = b.u32(B(6, 1), B(8, 65), B(10, 321), B(12, 1345))
            ratio = b.f(3)
            if ratio:
                n, d = ASPECT[ratio]
                self.xsize = self.ysize * n // d
            elif div8:
                self.xsize = b.u32(V(16), V(32), B(5, 1), B(9, 33)) * 8
            else:
                self.xsize = b.u32(B(6, 1), B(8, 65), B(10, 321), B(12, 1345))
        return self


class BitDepth:
    def read(self, b):
        self.float = b.boolean()
        if not self.float:
            self.bits = b.u32(V(8), V(10), V(12), B(6, 1))
            self.exp_bits = 0
        else:
            self.bits = b.u32(V(32), V(16), V(24), B(6, 1))
            self.exp_bits = b.f(4) + 1
        return self


class ExtraChannelInfo:
    ALPHA, DEPTH, SPOT, MASK, BLACK, CFA, THERMAL = 0, 1, 2, 3, 4, 5, 6

    def read(self, b):
        if b.boolean():         # all_default
            self.type = 0
            self.bit_depth = BitDepth()
            self.bit_depth.float, self.bit_depth.bits, self.bit_depth.exp_bits = False, 8, 0
            self.dim_shift = 0
            self.name = b""
            self.alpha_associated = False
            return self
        self.type = b.enum()
        self.bit_depth = BitDepth().read(b)
        self.dim_shift = b.u32(V(0), V(3), V(4), B(3, 1))
        n = b.u32(V(0), B(4), B(5, 16), B(10, 48))
        self.name = bytes(b.f(8) for _ in range(n))
        self.alpha_associated = False
        if self.type == self.ALPHA:
            self.alpha_associated = b.boolean()
        if self.type == self.SPOT:
            self.spot = [b.f16() for _ in range(4)]
        if self.type == self.CFA:
            self.cfa_channel = b.u32(V(1), B(2), B(4, 3), B(8, 19))
        if self.type > 6 and self.type != 15:
            raise JXLError("unknown extra channel type %d" % self.type)
        return self


class ColorEncoding:
    RGB, GRAY, XYB, UNKNOWN = 0, 1, 2, 3

    def __init__(self):
        self.want_icc = False
        self.color_space = 0
        self.white_point = 1    # D65
        self.primaries = 1      # sRGB
        self.have_gamma = False
        self.gamma = 0
        self.tf = 13            # sRGB
        self.intent = 1
        self.white = self.red = self.green = self.blue = None

    def read(self, b):
        if b.boolean():         # all_default: sRGB
            return self
        self.want_icc = b.boolean()
        self.color_space = b.enum()
        if not self.want_icc:
            if self.color_space != self.XYB:
                self.white_point = b.enum()
                if self.white_point == 2:
                    self.white = self.customxy(b)
            if self.color_space not in (self.GRAY, self.XYB):
                self.primaries = b.enum()
                if self.primaries == 2:
                    self.red, self.green, self.blue = self.customxy(b), self.customxy(b), self.customxy(b)
            if self.color_space != self.XYB:      # CustomTransferFunction (implicit for XYB)
                self.have_gamma = b.boolean()
                if self.have_gamma:
                    self.gamma = b.f(24)
                else:
                    self.tf = b.enum()
            else:
                self.have_gamma, self.gamma = True, 3333333
            self.intent = b.enum()
        return self

    @staticmethod
    def customxy(b):
        d = (B(19), B(19, 524288), B(20, 1048576), B(21, 2097152))
        return unpack_signed(b.u32(*d)), unpack_signed(b.u32(*d))


class ToneMapping:
    def read(self, b):
        self.intensity_target, self.min_nits, self.relative_to_max, self.linear_below = 255.0, 0.0, False, 0.0
        if b.boolean():
            return self
        self.intensity_target = b.f16()
        self.min_nits = b.f16()
        self.relative_to_max = b.boolean()
        self.linear_below = b.f16()
        return self


class ImageMetadata:
    def read(self, b):
        self.orientation = 1
        self.have_preview = self.have_animation = self.have_intrinsic = False
        self.bit_depth = BitDepth()
        self.bit_depth.float, self.bit_depth.bits, self.bit_depth.exp_bits = False, 8, 0
        self.modular_16bit = True
        self.extra = []
        self.xyb_encoded = True
        self.color = ColorEncoding()
        self.tone = ToneMapping()
        self.tone.intensity_target, self.tone.min_nits, self.tone.relative_to_max, self.tone.linear_below = \
            255.0, 0.0, False, 0.0
        self.animation = None
        if b.boolean():         # all_default
            return self
        extra_fields = b.boolean()
        if extra_fields:
            self.orientation = b.f(3) + 1
            self.have_intrinsic = b.boolean()
            if self.have_intrinsic:
                self.intrinsic = SizeHeader().read(b)
            self.have_preview = b.boolean()
            if self.have_preview:
                self.preview = SizeHeader().read(b, preview=True)
            self.have_animation = b.boolean()
            if self.have_animation:
                a = {}
                a["tps_num"] = b.u32(V(100), V(1000), B(10, 1), B(30, 1))
                a["tps_den"] = b.u32(V(1), V(1001), B(8, 1), B(10, 1))
                a["num_loops"] = b.u32(V(0), B(3), B(16), B(32))
                a["have_timecodes"] = b.boolean()
                self.animation = a
        self.bit_depth = BitDepth().read(b)
        self.modular_16bit = b.boolean()
        n = b.u32(V(0), V(1), B(4, 2), B(12, 1))
        self.extra = [ExtraChannelInfo().read(b) for _ in range(n)]
        self.xyb_encoded = b.boolean()
        self.color = ColorEncoding().read(b)
        if extra_fields:
            self.tone = ToneMapping().read(b)
        b.extensions()
        return self


# default transform data
DEFAULT_INV_OPSIN = [[11.031566901960783, -9.866943921568629, -0.16462299647058826],
                     [-3.254147380392157, 4.418770392156863, -0.16462299647058826],
                     [-3.6588512862745097, 2.7129230470588235, 1.9459282392156863]]
DEFAULT_OPSIN_BIAS = [-0.0037930732552754493] * 3
DEFAULT_QUANT_BIAS = [1.0 - 0.05465007330715401, 1.0 - 0.07005449891748593, 1.0 - 0.049935103337343655,
                      0.145]

UP2_W = [-0.01716200, -0.03452303, -0.04022174, -0.02921014, -0.00624645, 0.14111091, 0.28896755, 0.00278718,
         -0.01610267, 0.56661550, 0.03777607, -0.01986694, -0.03144731, -0.01185068, -0.00213539]


class TransformData:
    def read(self, b, xyb):
        self.inv_opsin = [list(r) for r in DEFAULT_INV_OPSIN]
        self.opsin_bias = list(DEFAULT_OPSIN_BIAS)
        self.quant_bias = list(DEFAULT_QUANT_BIAS)
        self.custom_weights = 0
        self.up_weights = {}
        if b.boolean():
            return self
        if xyb:
            if not b.boolean():         # OpsinInverseMatrix not all default
                for j in range(3):
                    for i in range(3):
                        self.inv_opsin[j][i] = b.f16()
                for i in range(3):
                    self.opsin_bias[i] = b.f16()
                for i in range(4):
                    self.quant_bias[i] = b.f16()
        self.custom_weights = b.f(3)
        for bit, n in ((1, 15), (2, 55), (4, 210)):
            if self.custom_weights & bit:
                self.up_weights[bit] = [b.f16() for _ in range(n)]
        return self


# ====================================================================== entropy decoding

ANS_LOG_TAB_SIZE = 12
ANS_TAB_SIZE = 1 << 12
ANS_SIGNATURE = 0x13
PREFIX_MAX_BITS = 15
WINDOW = 1 << 20

SPECIAL_DIST = [
    (0, 1), (1, 0), (1, 1), (-1, 1), (0, 2), (2, 0), (1, 2), (-1, 2), (2, 1), (-2, 1), (2, 2), (-2, 2), (0, 3),
    (3, 0), (1, 3), (-1, 3), (3, 1), (-3, 1), (2, 3), (-2, 3), (3, 2), (-3, 2), (0, 4), (4, 0), (1, 4), (-1, 4),
    (4, 1), (-4, 1), (3, 3), (-3, 3), (2, 4), (-2, 4), (4, 2), (-4, 2), (0, 5), (3, 4), (-3, 4), (4, 3), (-4, 3),
    (5, 0), (1, 5), (-1, 5), (5, 1), (-5, 1), (2, 5), (-2, 5), (5, 2), (-5, 2), (4, 4), (-4, 4), (3, 5), (-3, 5),
    (5, 3), (-5, 3), (0, 6), (6, 0), (1, 6), (-1, 6), (6, 1), (-6, 1), (2, 6), (-2, 6), (6, 2), (-6, 2), (4, 5),
    (-4, 5), (5, 4), (-5, 4), (3, 6), (-3, 6), (6, 3), (-6, 3), (0, 7), (7, 0), (1, 7), (-1, 7), (5, 5), (-5, 5),
    (7, 1), (-7, 1), (4, 6), (-4, 6), (6, 4), (-6, 4), (2, 7), (-2, 7), (7, 2), (-7, 2), (3, 7), (-3, 7), (7, 3),
    (-7, 3), (5, 6), (-5, 6), (6, 5), (-6, 5), (8, 0), (4, 7), (-4, 7), (7, 4), (-7, 4), (8, 1), (8, 2), (6, 6),
    (-6, 6), (8, 3), (5, 7), (-5, 7), (7, 5), (-7, 5), (8, 4), (6, 7), (-6, 7), (7, 6), (-7, 6), (8, 5), (7, 7),
    (-7, 7), (8, 6), (8, 7)]

# huff[128][2] of ReadHistogram: (bits consumed, log count) for the 7-bit peek
HIST_HUFF = [
    (3, 10), (7, 12), (3, 7), (4, 3), (3, 6), (3, 8), (3, 9), (4, 5), (3, 10), (4, 4), (3, 7), (4, 1), (3, 6),
    (3, 8), (3, 9), (4, 2), (3, 10), (5, 0), (3, 7), (4, 3), (3, 6), (3, 8), (3, 9), (4, 5), (3, 10), (4, 4),
    (3, 7), (4, 1), (3, 6), (3, 8), (3, 9), (4, 2), (3, 10), (6, 11), (3, 7), (4, 3), (3, 6), (3, 8), (3, 9),
    (4, 5), (3, 10), (4, 4), (3, 7), (4, 1), (3, 6), (3, 8), (3, 9), (4, 2), (3, 10), (5, 0), (3, 7), (4, 3),
    (3, 6), (3, 8), (3, 9), (4, 5), (3, 10), (4, 4), (3, 7), (4, 1), (3, 6), (3, 8), (3, 9), (4, 2), (3, 10),
    (7, 13), (3, 7), (4, 3), (3, 6), (3, 8), (3, 9), (4, 5), (3, 10), (4, 4), (3, 7), (4, 1), (3, 6), (3, 8),
    (3, 9), (4, 2), (3, 10), (5, 0), (3, 7), (4, 3), (3, 6), (3, 8), (3, 9), (4, 5), (3, 10), (4, 4), (3, 7),
    (4, 1), (3, 6), (3, 8), (3, 9), (4, 2), (3, 10), (6, 11), (3, 7), (4, 3), (3, 6), (3, 8), (3, 9), (4, 5),
    (3, 10), (4, 4), (3, 7), (4, 1), (3, 6), (3, 8), (3, 9), (4, 2), (3, 10), (5, 0), (3, 7), (4, 3), (3, 6),
    (3, 8), (3, 9), (4, 5), (3, 10), (4, 4), (3, 7), (4, 1), (3, 6), (3, 8), (3, 9), (4, 2)]


def floor_log2(x):
    return x.bit_length() - 1


def ceil_log2(x):
    return 0 if x <= 1 else (x - 1).bit_length()


def varlen_u8(b):
    if b.f(1):
        n = b.f(3)
        return 1 if n == 0 else b.f(n) + (1 << n)
    return 0


def varlen_u16(b):
    if b.f(1):
        n = b.f(4)
        return 1 if n == 0 else b.f(n) + (1 << n)
    return 0


def pop_precision(logcount, shift):
    r = min(logcount, shift - ((ANS_LOG_TAB_SIZE - logcount) >> 1))
    return max(0, r)


def flat_histogram(length, total):
    c = total // length
    res = [c] * length
    for i in range(total % length):
        res[i] += 1
    return res


def read_histogram(b, precision_bits=ANS_LOG_TAB_SIZE):
    rng = 1 << precision_bits
    if b.f(1):          # simple code
        n = b.f(1) + 1
        syms = [varlen_u8(b) for _ in range(n)]
        counts = [0] * (max(syms) + 1)
        if n == 1:
            counts[syms[0]] = rng
        else:
            if syms[0] == syms[1]:
                raise JXLError("histogram: equal symbols")
            counts[syms[0]] = b.f(precision_bits)
            counts[syms[1]] = rng - counts[syms[0]]
        return counts
    if b.f(1):          # flat
        return flat_histogram(varlen_u8(b) + 1, rng)
    upper = floor_log2(ANS_LOG_TAB_SIZE + 1)
    log = 0
    while log < upper:
        if b.f(1) == 0:
            break
        log += 1
    shift = (b.f(log) | (1 << log)) - 1
    if shift > ANS_LOG_TAB_SIZE + 1:
        raise JXLError("histogram shift")
    length = varlen_u8(b) + 3
    counts = [0] * length
    logcounts = [0] * length
    same = [0] * length
    omit_log, omit_pos = -1, -1
    i = 0
    while i < length:
        idx = b.peek(7)
        nb, lc = HIST_HUFF[idx]
        b.f(nb)
        logcounts[i] = lc
        if lc == ANS_LOG_TAB_SIZE + 1:
            rle = varlen_u8(b)
            same[i] = rle + 5
            i += rle + 3 + 1
            continue
        if lc > omit_log:
            omit_log, omit_pos = lc, i
        i += 1
    if omit_pos < 0:
        raise JXLError("histogram: no omit")
    if omit_pos + 1 < length and logcounts[omit_pos + 1] == ANS_TAB_SIZE + 1:
        raise JXLError("histogram")
    total = 0
    prev = 0
    numsame = 0
    for i in range(length):
        if same[i]:
            numsame = same[i] - 1
            prev = counts[i - 1] if i > 0 else 0
        if numsame > 0:
            counts[i] = prev
            numsame -= 1
        else:
            code = logcounts[i]
            if i == omit_pos or code == 0:
                continue
            if code == 1:
                counts[i] = 1
            else:
                bc = pop_precision(code - 1, shift)
                counts[i] = (1 << (code - 1)) + (b.f(bc) << (code - 1 - bc))
        total += counts[i]
    counts[omit_pos] = rng - total
    if counts[omit_pos] <= 0:
        raise JXLError("histogram count")
    return counts


def init_alias(dist, log_range, log_alpha):
    """InitAliasTable -> list of (cutoff, right_value, freq0, offsets1, freq1)"""
    rng = 1 << log_range
    tsize = 1 << log_alpha
    dist = list(dist)
    while dist and dist[-1] == 0:
        dist.pop()
    if not dist:
        dist = [rng]
    if len(dist) > tsize:
        raise JXLError("alphabet too large")
    esize = rng >> log_alpha
    single = -1
    for s, v in enumerate(dist):
        if v == ANS_TAB_SIZE:
            single = s
    if sum(dist) != rng:
        raise JXLError("histogram sum")
    if single != -1:
        return [(0, single, 0, esize * i, ANS_TAB_SIZE) for i in range(tsize)]
    under, over = [], []
    cutoffs = [0] * tsize
    right = [0] * tsize
    offs = [0] * tsize
    for i, v in enumerate(dist):
        cutoffs[i] = v
        if v > esize:
            over.append(i)
        elif v < esize:
            under.append(i)
    for i in range(len(dist), tsize):
        cutoffs[i] = 0
        under.append(i)
    while over:
        o = over.pop()
        u = under.pop()
        by = esize - cutoffs[u]
        cutoffs[o] -= by
        right[u] = o
        offs[u] = cutoffs[o]
        if cutoffs[o] < esize:
            under.append(o)
        elif cutoffs[o] > esize:
            over.append(o)
    table = []
    for i in range(tsize):
        if cutoffs[i] == esize:
            right[i], offs[i], cut = i, 0, 0
        else:
            offs[i] -= cutoffs[i]
            cut = cutoffs[i]
        f0 = dist[i] if i < len(dist) else 0
        r = right[i]
        f1 = dist[r] if r < len(dist) else 0
        table.append((cut, r, f0, offs[i], f1))
    return table


class Prefix:
    """canonical prefix code (Brotli style, JXL bit order): decoded bit by bit"""

    def __init__(self, lengths):
        self.single = None
        nz = [s for s, l in enumerate(lengths) if l]
        if len(nz) == 1 or not nz:
            self.single = nz[0] if nz else 0
            return
        order = sorted(nz, key=lambda s: (lengths[s], s))
        self.codes = {}
        code = 0
        prevlen = lengths[order[0]]
        for s in order:
            l = lengths[s]
            code <<= (l - prevlen)
            prevlen = l
            self.codes[(l, code)] = s
            code += 1
        self.maxlen = max(lengths)

    def read(self, b):
        if self.single is not None:
            return self.single
        code = 0
        for l in range(1, self.maxlen + 1):
            code = (code << 1) | b.f(1)
            s = self.codes.get((l, code))
            if s is not None:
                return s
        raise JXLError("invalid prefix code")


CL_ORDER = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15]
# static code for the code length code lengths: (bits, value) by 4-bit peek
CL_HUFF = [(2, 0), (2, 4), (2, 3), (3, 2), (2, 0), (2, 4), (2, 3), (4, 1),
           (2, 0), (2, 4), (2, 3), (3, 2), (2, 0), (2, 4), (2, 3), (4, 5)]


def read_prefix(b, alphabet_size):
    sel = b.f(2)
    if sel == 1:        # simple code
        maxbits = floor_log2(alphabet_size - 1) + 1 if alphabet_size > 1 else 0
        n = b.f(2) + 1
        syms = [b.f(maxbits) for _ in range(n)]
        for s in syms:
            if s >= alphabet_size:
                raise JXLError("prefix symbol")
        if len(set(syms)) != n:
            raise JXLError("prefix: duplicate")
        if n == 4:
            n += b.f(1)
        lengths = [0] * alphabet_size
        if n == 1:
            lengths[syms[0]] = 0
            p = Prefix(lengths)
            p.single = syms[0]
            return p
        if n == 2:
            for s in syms:
                lengths[s] = 1
        elif n == 3:
            lengths[syms[0]] = 1
            lengths[syms[1]] = lengths[syms[2]] = 2
        elif n == 4:
            for s in syms:
                lengths[s] = 2
        else:
            s3 = sorted(syms[2:4])
            lengths[syms[0]] = 1
            lengths[syms[1]] = 2
            lengths[s3[0]] = lengths[s3[1]] = 3
        return Prefix(lengths)
    cl = [0] * 18
    space = 32
    ncodes = 0
    i = sel
    while i < 18 and space > 0:
        idx = CL_ORDER[i]
        nb, v = CL_HUFF[b.peek(4)]
        b.f(nb)
        cl[idx] = v
        if v:
            space -= 32 >> v
            ncodes += 1
        i += 1
    if not (ncodes == 1 or space == 0):
        raise JXLError("prefix code length code")
    clc = Prefix(cl)
    lengths = [0] * alphabet_size
    sym = 0
    prev = 8
    repeat = 0
    repeat_len = 0
    space = 32768
    while sym < alphabet_size and space > 0:
        c = clc.read(b)
        if c < 16:
            repeat = 0
            lengths[sym] = c
            sym += 1
            if c:
                prev = c
                space -= 32768 >> c
        else:
            extra = c - 14
            new_len = prev if c == 16 else 0
            if repeat_len != new_len:
                repeat = 0
                repeat_len = new_len
            old = repeat
            if repeat > 0:
                repeat -= 2
                repeat <<= extra
            repeat += b.f(extra) + 3
            delta = repeat - old
            if sym + delta > alphabet_size:
                raise JXLError("prefix repeat")
            for k in range(delta):
                lengths[sym + k] = repeat_len
            sym += delta
            if repeat_len:
                space -= delta << (15 - repeat_len)
    if space != 0:
        raise JXLError("prefix code space")
    return Prefix(lengths)


class HybridUint:
    def __init__(self, split_exponent=4, msb=2, lsb=0):
        self.se, self.msb, self.lsb = split_exponent, msb, lsb
        self.split = 1 << split_exponent

    def read(self, token, b):
        if token < self.split:
            return token
        n = self.se - (self.msb + self.lsb) + ((token - self.split) >> (self.msb + self.lsb))
        n &= 31
        low = token & ((1 << self.lsb) - 1)
        token >>= self.lsb
        bits = b.f(n)
        return ((((1 << self.msb) | (token & ((1 << self.msb) - 1))) << n | bits) << self.lsb) | low


def read_uint_config(b, log_alpha):
    se = b.f(ceil_log2(log_alpha + 1))
    msb = lsb = 0
    if se != log_alpha:
        msb = b.f(ceil_log2(se + 1))
        if msb > se:
            raise JXLError("uint config")
        lsb = b.f(ceil_log2(se - msb + 1))
    if lsb + msb > se:
        raise JXLError("uint config")
    return HybridUint(se, msb, lsb)


class Code:
    """DecodeHistograms result: context map, prefix codes or ANS alias tables, configs, LZ77"""


def decode_histograms(b, num_contexts, disallow_lz77=False):
    c = Code()
    c.lz77 = b.boolean()
    if c.lz77:
        c.lz77_min_symbol = b.u32(V(224), V(512), V(4096), B(15, 8))
        c.lz77_min_length = b.u32(V(3), V(4), B(2, 5), B(8, 9))
        num_contexts += 1
        c.lz77_length = read_uint_config(b, 8)
        if disallow_lz77:
            raise JXLError("LZ77 not allowed")
    if num_contexts > 1:
        c.ctx_map = decode_context_map(b, num_contexts)
    else:
        c.ctx_map = [0]
    nh = max(c.ctx_map) + 1
    c.lz77_ctx = c.ctx_map[-1]
    c.prefix = b.boolean()
    c.log_alpha = PREFIX_MAX_BITS if c.prefix else b.f(2) + 5
    c.configs = [read_uint_config(b, c.log_alpha) for _ in range(nh)]
    if c.prefix:
        sizes = [varlen_u16(b) + 1 for _ in range(nh)]
        c.codes = []
        for s in sizes:
            if s > 1:
                c.codes.append(read_prefix(b, s))
            else:
                p = Prefix([0])
                p.single = 0
                c.codes.append(p)
    else:
        c.tables = []
        for _ in range(nh):
            counts = read_histogram(b)
            if len(counts) > (1 << c.log_alpha):
                raise JXLError("alphabet size")
            c.tables.append(init_alias(counts, ANS_LOG_TAB_SIZE, c.log_alpha))
    return c


def decode_context_map(b, n):
    if b.f(1):          # simple
        bpe = b.f(2)
        cm = [b.f(bpe) for _ in range(n)] if bpe else [0] * n
    else:
        use_mtf = b.f(1)
        code = decode_histograms(b, 1, disallow_lz77=n <= 2)
        r = Reader(code, b)
        cm = [r.read(0) for _ in range(n)]
        if max(cm) >= 256:
            raise JXLError("cluster id")
        if not r.final_ok():
            raise JXLError("context map ANS state")
        if use_mtf:
            mtf = list(range(256))
            for i in range(n):
                idx = cm[i]
                v = mtf[idx]
                cm[i] = v
                if idx:
                    del mtf[idx]
                    mtf.insert(0, v)
    nh = max(cm) + 1
    if len(set(cm)) != nh:
        raise JXLError("incomplete context map")
    return cm


class Reader:
    """ANSSymbolReader: ReadHybridUint( ctx ) through the context map"""

    def __init__(self, code, b, dist_mult=0):
        self.c = code
        self.b = b
        if not code.prefix:
            self.state = b.f(32)
            self.log_entry = ANS_LOG_TAB_SIZE - code.log_alpha
        else:
            self.state = ANS_SIGNATURE << 16
        self.window = None
        if code.lz77:
            self.window = [0] * WINDOW
            self.num_to_copy = 0
            self.copy_pos = 0
            self.num_decoded = 0
            self.special = [max(1, d0 + dist_mult * d1) for d0, d1 in SPECIAL_DIST] if dist_mult else []

    def symbol(self, h):
        c = self.c
        if c.prefix:
            return c.codes[h].read(self.b)
        res = self.state & (ANS_TAB_SIZE - 1)
        tab = c.tables[h]
        i = res >> self.log_entry
        pos = res & ((1 << self.log_entry) - 1)
        cut, right, f0, off1, f1 = tab[i]
        if pos >= cut:
            val, off, freq = right, off1 + pos, f1
        else:
            val, off, freq = i, pos, f0
        self.state = freq * (self.state >> ANS_LOG_TAB_SIZE) + off
        if self.state < (1 << 16):
            self.state = ((self.state << 16) | self.b.f(16)) & 0xFFFFFFFF
        return val

    def read_clustered(self, h):
        c = self.c
        if self.window is not None and self.num_to_copy > 0:
            v = self.window[self.copy_pos & (WINDOW - 1)]
            self.copy_pos += 1
            self.num_to_copy -= 1
            self.window[self.num_decoded & (WINDOW - 1)] = v
            self.num_decoded += 1
            return v
        tok = self.symbol(h)
        if self.window is not None and tok >= c.lz77_min_symbol:
            self.num_to_copy = c.lz77_length.read(tok - c.lz77_min_symbol, self.b) + c.lz77_min_length
            t2 = self.symbol(c.lz77_ctx)
            dist = c.configs[c.lz77_ctx].read(t2, self.b)
            ns = len(self.special)
            if dist < ns:
                dist = self.special[dist]
            else:
                dist = dist + 1 - ns
            dist = min(dist, self.num_decoded, WINDOW)
            self.copy_pos = self.num_decoded - dist
            if dist == 0:
                for k in range(min(self.num_to_copy, WINDOW)):
                    self.window[k] = 0
            v = self.window[self.copy_pos & (WINDOW - 1)]
            self.copy_pos += 1
            self.num_to_copy -= 1
            self.window[self.num_decoded & (WINDOW - 1)] = v
            self.num_decoded += 1
            return v
        v = c.configs[h].read(tok, self.b)
        if self.window is not None:
            self.window[self.num_decoded & (WINDOW - 1)] = v
            self.num_decoded += 1
        return v

    def read(self, ctx):
        return self.read_clustered(self.c.ctx_map[ctx])

    def final_ok(self):
        return self.state == (ANS_SIGNATURE << 16)


# ====================================================================== ICC

ICC_HEADER_PRED = bytes([
    0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, ord('m'), ord('n'), ord('t'), ord('r'),
    ord('R'), ord('G'), ord('B'), ord(' '), ord('X'), ord('Y'), ord('Z'), ord(' '), 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, ord('a'), ord('c'), ord('s'), ord('p'), 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 246, 214, 0, 1, 0, 0, 0, 0, 211, 45,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
TAG_STRINGS = [b"cprt", b"wtpt", b"bkpt", b"rXYZ", b"gXYZ", b"bXYZ", b"kXYZ", b"rTRC", b"gTRC", b"bTRC", b"kTRC",
               b"chad", b"desc", b"chrm", b"dmnd", b"dmdd", b"lumi"]
TYPE_STRINGS = [b"XYZ ", b"desc", b"text", b"mluc", b"para", b"curv", b"sf32", b"gbd "]


def byte_kind1(b):
    if 97 <= b <= 122 or 65 <= b <= 90:
        return 0
    if 48 <= b <= 57 or b in (46, 44):
        return 1
    if b == 0:
        return 2
    if b == 1:
        return 3
    if b < 16:
        return 4
    if b == 255:
        return 6
    if b > 240:
        return 5
    return 7


def byte_kind2(b):
    if 97 <= b <= 122 or 65 <= b <= 90:
        return 0
    if 48 <= b <= 57 or b in (46, 44):
        return 1
    if b < 16:
        return 2
    if b > 240:
        return 3
    return 4


def icc_ctx(i, b1, b2):
    if i <= 128:
        return 0
    return 1 + byte_kind1(b1) + byte_kind2(b2) * 8


def read_icc(b):
    enc_size = b.u64()
    if enc_size > 1 << 28:
        raise JXLError("ICC too large")
    code = decode_histograms(b, 41)
    r = Reader(code, b)
    dec = bytearray(enc_size)
    for i in range(enc_size):
        dec[i] = r.read(icc_ctx(i, dec[i - 1] if i > 0 else 0, dec[i - 2] if i > 1 else 0)) & 255
    if not r.final_ok():
        raise JXLError("ICC ANS state")
    return unpredict_icc(bytes(dec))


def varint(d, pos):
    v = 0
    i = 0
    while pos + i < len(d) and i < 10:
        v |= (d[pos + i] & 127) << (7 * i)
        if not d[pos + i] & 128:
            break
        i += 1
    return v, pos + i + 1


def icc_shuffle(data, width):
    size = len(data)
    height = (size + width - 1) // width
    res = bytearray(size)
    s = j = 0
    for i in range(size):
        res[i] = data[j]
        j += height
        if j >= size:
            s += 1
            j = s
    return res


def icc_predict(res, start, i, stride, width, order):
    def pv(p1, p2, p3):
        return p1 if order == 0 else (2 * p1 - p2) if order == 1 else (3 * p1 - 3 * p2 + p3)
    pos = start + i
    if width == 1:
        return pv(res[pos - stride], res[pos - 2 * stride], res[pos - 3 * stride]) & 255
    if width == 2:
        p = start + (i & ~1)
        vals = [(res[p - stride * k] << 8) + res[p - stride * k + 1] for k in (1, 2, 3)]
        pred = pv(*vals) & 0xFFFF
        return pred & 255 if i & 1 else (pred >> 8) & 255
    p = start + (i & ~3)

    def be32(q):
        return 0 if q + 4 > pos else int.from_bytes(res[q:q + 4], "big")
    pred = pv(be32(p - stride), be32(p - 2 * stride), be32(p - 3 * stride)) & 0xFFFFFFFF
    return (pred >> ((3 - (i & 3)) * 8)) & 255


def unpredict_icc(enc):
    size = len(enc)
    osize, pos = varint(enc, 0)
    csize, pos = varint(enc, pos)
    cpos = pos
    cend = cpos + csize
    if cend > size:
        raise JXLError("ICC commands")
    pos = cend
    header = bytearray(ICC_HEADER_PRED)
    header[0:4] = osize.to_bytes(4, "big")
    res = bytearray()
    for i in range(129):
        if len(res) == osize:
            return bytes(res)
        if i == 128:
            break
        if i == 8 and len(res) >= 8:
            header[80:84] = res[4:8]
        if i == 41 and len(res) >= 41:
            if res[40] == ord('A'):
                header[41:44] = b"PPL"
            if res[40] == ord('M'):
                header[41:44] = b"SFT"
        if i == 42 and len(res) >= 42:
            if res[40] == ord('S') and res[41] == ord('G'):
                header[42:44] = b"I "
            if res[40] == ord('S') and res[41] == ord('U'):
                header[42:44] = b"NW"
        res.append((enc[pos] + header[i]) & 255)
        pos += 1
    numtags, cpos = varint(enc, cpos)
    if numtags:
        numtags -= 1
        res += numtags.to_bytes(4, "big")
        prevstart = 128 + numtags * 12
        prevsize = 0
        while cpos < cend:
            cmd = enc[cpos]
            cpos += 1
            tc = cmd & 63
            if tc == 0:
                break
            if tc == 1:
                tag = bytes(enc[pos:pos + 4])
                pos += 4
            elif tc == 2:
                tag = b"rTRC"
            elif tc == 3:
                tag = b"rXYZ"
            else:
                tag = TAG_STRINGS[tc - 4]
            res += tag
            tsize = prevsize
            if tag in (b"rXYZ", b"gXYZ", b"bXYZ", b"kXYZ", b"wtpt", b"bkpt", b"lumi"):
                tsize = 20
            if cmd & 64:
                tstart, cpos = varint(enc, cpos)
            else:
                tstart = prevstart + prevsize
            res += tstart.to_bytes(4, "big")
            if cmd & 128:
                tsize, cpos = varint(enc, cpos)
            res += tsize.to_bytes(4, "big")
            prevstart, prevsize = tstart, tsize
            if tc == 2:
                res += b"gTRC" + tstart.to_bytes(4, "big") + tsize.to_bytes(4, "big")
                res += b"bTRC" + tstart.to_bytes(4, "big") + tsize.to_bytes(4, "big")
            if tc == 3:
                res += b"gXYZ" + (tstart + tsize).to_bytes(4, "big") + tsize.to_bytes(4, "big")
                res += b"bXYZ" + (tstart + 2 * tsize).to_bytes(4, "big") + tsize.to_bytes(4, "big")
    while cpos < cend:
        cmd = enc[cpos]
        cpos += 1
        if cmd == 1:
            num, cpos = varint(enc, cpos)
            res += enc[pos:pos + num]
            pos += num
        elif cmd in (2, 3):
            num, cpos = varint(enc, cpos)
            res += icc_shuffle(enc[pos:pos + num], 2 if cmd == 2 else 4)
            pos += num
        elif cmd == 4:
            flags = enc[cpos]
            cpos += 1
            width = (flags & 3) + 1
            order = (flags & 12) >> 2
            if width == 3 or order == 3:
                raise JXLError("ICC predict")
            stride = width
            if flags & 16:
                stride, cpos = varint(enc, cpos)
            num, cpos = varint(enc, cpos)
            data = enc[pos:pos + num]
            if width > 1:
                data = icc_shuffle(data, width)
            start = len(res)
            for i in range(num):
                res.append((icc_predict(res, start, i, stride, width, order) + data[i]) & 255)
            pos += num
        elif cmd == 10:
            res += b"XYZ " + bytes(4) + enc[pos:pos + 12]
            pos += 12
        elif 16 <= cmd < 24:
            res += TYPE_STRINGS[cmd - 16] + bytes(4)
        else:
            raise JXLError("ICC command %d" % cmd)
    if len(res) != osize:
        raise JXLError("ICC size")
    return bytes(res)


# ====================================================================== frame header

REGULAR, DC_FRAME, REFERENCE_ONLY, SKIP_PROGRESSIVE = 0, 1, 2, 3
BM_REPLACE, BM_ADD, BM_BLEND, BM_AWADD, BM_MUL = 0, 1, 2, 3, 4
F_NOISE, F_PATCHES, F_SPLINES, F_USE_DC_FRAME, F_SKIP_ADAPTIVE_DC = 1, 2, 16, 32, 128
CS_HSHIFT = [0, 1, 1, 0]
CS_VSHIFT = [0, 1, 0, 1]


class Blending:
    def read(self, b, num_ec, partial):
        self.mode = b.u32(V(0), V(1), V(2), B(2, 3))
        self.alpha_channel, self.clamp, self.source = 0, False, 0
        if num_ec > 0 and self.mode in (BM_BLEND, BM_AWADD):
            self.alpha_channel = b.u32(V(0), V(1), V(2), B(3, 3))
        if (num_ec > 0 and self.mode in (BM_BLEND, BM_AWADD)) or self.mode == BM_MUL:
            self.clamp = b.boolean()
        if self.mode != BM_REPLACE or partial:
            self.source = b.u32(V(0), V(1), V(2), V(3))
        return self


class LoopFilter:
    def read(self, b, modular):
        self.gab = True
        self.gab_w = [(0.115169525, 0.061248592)] * 3
        self.epf_iters = 2
        self.epf_sharp_lut = [i / 7.0 for i in range(8)]
        self.epf_channel_scale = [40.0, 5.0, 3.5]
        self.epf_pass1_zeroflush, self.epf_pass2_zeroflush = 0.45, 0.6
        self.epf_quant_mul = 0.46
        self.epf_pass0_sigma_scale, self.epf_pass2_sigma_scale = 0.9, 6.5
        self.epf_border_sad_mul = 0.6666666666666666
        self.epf_sigma_for_modular = 1.0
        if b.boolean():         # all_default
            return self
        self.gab = b.boolean()
        if self.gab and b.boolean():
            self.gab_w = [(b.f16(), b.f16()) for _ in range(3)]
        self.epf_iters = b.f(2)
        if self.epf_iters:
            if not modular and b.boolean():
                self.epf_sharp_lut = [b.f16() for _ in range(8)]
            if b.boolean():
                self.epf_channel_scale = [b.f16(), b.f16(), b.f16()]
                self.epf_pass1_zeroflush = b.f16()
                self.epf_pass2_zeroflush = b.f16()
            if b.boolean():
                if not modular:
                    self.epf_quant_mul = b.f16()
                self.epf_pass0_sigma_scale = b.f16()
                self.epf_pass2_sigma_scale = b.f16()
                self.epf_border_sad_mul = b.f16()
            if modular:
                self.epf_sigma_for_modular = b.f16()
        b.extensions()
        return self


class FrameHeader:
    def read(self, b, meta, size, is_preview=False):
        num_ec = len(meta.extra)
        self.frame_type = REGULAR
        self.modular = False
        self.flags = 0
        self.ycbcr = False
        self.chroma_modes = [0, 0, 0]
        self.upsampling = 1
        self.ec_upsampling = [1 << e.dim_shift for e in meta.extra]
        self.group_size_shift = 1
        self.x_qm_scale, self.b_qm_scale = 3, 2
        self.num_passes, self.num_downsample = 1, 0
        self.shift = [0] * 11
        self.downsample, self.last_pass = [], []
        self.dc_level = 0
        self.x0 = self.y0 = 0
        self.xsize, self.ysize = (meta.preview.xsize, meta.preview.ysize) if is_preview else (size.xsize, size.ysize)
        self.custom_size = False
        self.blending = Blending()
        self.blending.mode, self.blending.alpha_channel, self.blending.clamp, self.blending.source = 0, 0, False, 0
        self.ec_blending = []
        self.duration = 0
        self.timecode = 0
        self.is_last = True
        self.save_as_reference = 0
        self.save_before_ct = False
        self.name = b""
        self.all_default = b.boolean()
        if self.all_default:
            self.lf = LoopFilter().read(Bits(b"\x01"), False)
            self.color_transform = 'xyb' if meta.xyb_encoded else 'none'
            self.finish(meta)
            return self
        self.frame_type = b.u32(V(0), V(1), V(2), V(3))
        self.modular = b.boolean()
        self.flags = b.u64()
        if meta.xyb_encoded:
            self.color_transform = 'xyb'
        else:
            self.color_transform = 'ycbcr' if b.boolean() else 'none'
        if self.color_transform == 'ycbcr' and not self.flags & F_USE_DC_FRAME:
            self.chroma_modes = [b.f(2) for _ in range(3)]
        if not self.flags & F_USE_DC_FRAME:
            self.upsampling = b.u32(V(1), V(2), V(4), V(8))
            for i, e in enumerate(meta.extra):
                u = b.u32(V(1), V(2), V(4), V(8))
                self.ec_upsampling[i] = u << e.dim_shift
                if self.ec_upsampling[i] < self.upsampling or self.ec_upsampling[i] > 8:
                    raise JXLError("extra channel upsampling")
        if self.modular:
            self.group_size_shift = b.f(2)
        if not self.modular and self.color_transform == 'xyb':
            self.x_qm_scale = b.f(3)
            self.b_qm_scale = b.f(3)
        else:
            self.x_qm_scale = self.b_qm_scale = 2
        if self.frame_type != REFERENCE_ONLY:
            self.num_passes = b.u32(V(1), V(2), V(3), B(3, 4))
            if self.num_passes != 1:
                self.num_downsample = b.u32(V(0), V(1), V(2), B(1, 3))
                for i in range(self.num_passes - 1):
                    self.shift[i] = b.f(2)
                self.downsample = [b.u32(V(1), V(2), V(4), V(8)) for _ in range(self.num_downsample)]
                self.last_pass = [b.u32(V(0), V(1), V(2), B(3)) for _ in range(self.num_downsample)]
        if self.frame_type == DC_FRAME:
            self.dc_level = b.u32(V(1), V(2), V(3), V(4))
        partial = False
        if self.frame_type != DC_FRAME:
            self.custom_size = b.boolean()
            if self.custom_size:
                enc = (B(8), B(11, 256), B(14, 2304), B(30, 18688))
                if self.frame_type in (REGULAR, SKIP_PROGRESSIVE):
                    self.x0 = unpack_signed(b.u32(*enc))
                    self.y0 = unpack_signed(b.u32(*enc))
                fx = b.u32(*enc)
                fy = b.u32(*enc)
                if fx == 0 or fy == 0:
                    raise JXLError("frame size 0")
                iw, ih = self.xsize, self.ysize
                self.xsize, self.ysize = fx, fy
                if self.frame_type in (REGULAR, SKIP_PROGRESSIVE):
                    partial = self.x0 > 0 or self.y0 > 0 or fx + self.x0 < iw or fy + self.y0 < ih
        if self.frame_type in (REGULAR, SKIP_PROGRESSIVE):
            self.blending = Blending().read(b, num_ec, partial)
            self.ec_blending = [Blending().read(b, num_ec, partial) for _ in range(num_ec)]
            if meta.have_animation:
                self.duration = b.u32(V(0), V(1), B(8), B(32))
                if meta.animation["have_timecodes"]:
                    self.timecode = b.f(32)
            self.is_last = b.boolean()
        else:
            self.is_last = False
        if self.frame_type != DC_FRAME and not self.is_last:
            self.save_as_reference = b.u32(V(0), V(1), V(2), V(3))
        if self.frame_type != DC_FRAME:
            can_ref = (not self.is_last and self.frame_type != DC_FRAME
                       and (self.duration == 0 or self.save_as_reference != 0))
            if can_ref and self.blending.mode == BM_REPLACE and not partial and \
                    self.frame_type in (REGULAR, SKIP_PROGRESSIVE):
                self.save_before_ct = b.boolean()
            elif self.frame_type == REFERENCE_ONLY:
                self.save_before_ct = b.boolean()
        else:
            self.save_before_ct = True
        n = b.u32(V(0), B(4), B(5, 16), B(10, 48))
        self.name = bytes(b.f(8) for _ in range(n))
        self.lf = LoopFilter().read(b, self.modular)
        b.extensions()
        self.partial = partial
        self.finish(meta)
        return self

    def finish(self, meta):
        self.maxhs = max(CS_HSHIFT[m] for m in self.chroma_modes)
        self.maxvs = max(CS_VSHIFT[m] for m in self.chroma_modes)
        xs, ys = self.xsize, self.ysize
        if self.dc_level:
            xs = -(-xs // (1 << (3 * self.dc_level)))
            ys = -(-ys // (1 << (3 * self.dc_level)))
        self.group_dim = 128 << self.group_size_shift
        self.width_up, self.height_up = xs, ys
        self.width = -(-xs // self.upsampling)
        self.height = -(-ys // self.upsampling)
        self.xsize_blocks = -(-self.width // (8 << self.maxhs)) << self.maxhs
        self.ysize_blocks = -(-self.height // (8 << self.maxvs)) << self.maxvs
        self.xsize_groups = -(-self.width // self.group_dim)
        self.ysize_groups = -(-self.height // self.group_dim)
        self.xsize_dc_groups = -(-self.xsize_blocks // self.group_dim)
        self.ysize_dc_groups = -(-self.ysize_blocks // self.group_dim)
        self.num_groups = self.xsize_groups * self.ysize_groups
        self.num_dc_groups = self.xsize_dc_groups * self.ysize_dc_groups

    def hshift(self, c):
        return self.maxhs - CS_HSHIFT[self.chroma_modes[c]]

    def vshift(self, c):
        return self.maxvs - CS_VSHIFT[self.chroma_modes[c]]


def read_toc(b, n):
    """sizes in group order (permutation applied), offsets relative to the end of the TOC"""
    perm = None
    if b.f(1):
        code = decode_histograms(b, 8)
        r = Reader(code, b)
        end = r.read(min(7, perm_ctx(n)))
        if end > n:
            raise JXLError("TOC permutation size")
        lehmer = [0] * n
        last = 0
        for i in range(end):
            lehmer[i] = r.read(min(7, perm_ctx(last)))
            last = lehmer[i]
            if lehmer[i] >= n - i:
                raise JXLError("lehmer code")
        if not r.final_ok():
            raise JXLError("TOC ANS state")
        rem = list(range(n))
        perm = [rem.pop(l) for l in lehmer]
    b.byte_align()
    sizes = [b.u32(B(10), B(14, 1024), B(22, 17408), B(30, 4211712)) for _ in range(n)]
    b.byte_align()
    offs = []
    o = 0
    for s in sizes:
        offs.append(o)
        o += s
    if perm is not None:
        offs = [offs[i] for i in perm]
        sizes = [sizes[i] for i in perm]
    return sizes, offs


def perm_ctx(v):
    """CoeffOrderContext without the clamp: the token of HybridUintConfig( 0, 0, 0 )"""
    return v if v < 1 else floor_log2(v) + 1


def toc_entries(fh):
    if fh.num_groups == 1 and fh.num_passes == 1:
        return 1
    return 2 + fh.num_dc_groups + fh.num_groups * fh.num_passes


# ====================================================================== modular

NUM_QUANT_TABLES = 17
NUM_NONREF_PROPS = 16       # 2 static + 13 + 1 (weighted predictor)
WP_PROP = 15

WP_DIVLOOKUP = [
    16777216, 8388608, 5592405, 4194304, 3355443, 2796202, 2396745, 2097152, 1864135, 1677721, 1525201, 1398101,
    1290555, 1198372, 1118481, 1048576, 986895, 932067, 883011, 838860, 798915, 762600, 729444, 699050, 671088,
    645277, 621378, 599186, 578524, 559240, 541200, 524288, 508400, 493447, 479349, 466033, 453438, 441505,
    430185, 419430, 409200, 399457, 390167, 381300, 372827, 364722, 356962, 349525, 342392, 335544, 328965,
    322638, 316551, 310689, 305040, 299593, 294337, 289262, 284359, 279620, 275036, 270600, 266305, 262144]


def div2(v):
    """C++ integer division by 2 (truncation)"""
    return v // 2 if v >= 0 else -((-v) // 2)


def cdiv(a, b):
    """C++ integer division (truncation toward zero)"""
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def wrap32(v):
    v &= 0xFFFFFFFF
    return v - (1 << 32) if v & 0x80000000 else v


def clamped_gradient(n, w, l):
    m, M = min(n, w), max(n, w)
    grad = wrap32(n + w - l)
    if l > M:
        return m
    if l < m:
        return M
    return grad


class WPHeader:
    def read(self, b):
        self.p1C, self.p2C, self.p3Ca, self.p3Cb, self.p3Cc, self.p3Cd, self.p3Ce = 16, 10, 7, 7, 7, 0, 0
        self.w = [0xd, 0xc, 0xc, 0xc]
        if b.boolean():
            return self
        self.p1C, self.p2C, self.p3Ca, self.p3Cb, self.p3Cc, self.p3Cd, self.p3Ce = [b.f(5) for _ in range(7)]
        self.w = [b.f(4) for _ in range(4)]
        return self


class WPState:
    def __init__(self, hdr, xsize):
        self.h = hdr
        self.xs = xsize
        n = (xsize + 2) * 2
        self.pred_errors = [[0] * n for _ in range(4)]
        self.error = [0] * n
        self.prediction = [0] * 4
        self.pred = 0

    def error_weight(self, x, maxweight):
        shift = max(0, floor_log2(x + 1) - 5)
        return 4 + ((maxweight * WP_DIVLOOKUP[x >> shift]) >> shift)

    def weighted_average(self, p, w):
        ws = sum(w)
        lw = floor_log2(ws)
        w = [v >> (lw - 4) for v in w]
        ws = sum(w)
        sm = (ws >> 1) - 1
        for i in range(4):
            sm += p[i] * w[i]
        return (sm * WP_DIVLOOKUP[ws - 1]) >> 24

    def predict(self, x, y, N, W, NE, NW, NN, props):
        xs = self.xs
        cur = 0 if y & 1 else xs + 2
        prev = xs + 2 if y & 1 else 0
        pN = prev + x
        pNE = pN + 1 if x < xs - 1 else pN
        pNW = pN - 1 if x > 0 else pN
        h = self.h
        weights = []
        for i in range(4):
            e = self.pred_errors[i]
            weights.append(self.error_weight(e[pN] + e[pNE] + e[pNW], h.w[i]))
        N <<= 3
        W <<= 3
        NE <<= 3
        NW <<= 3
        NN <<= 3
        teW = 0 if x == 0 else self.error[cur + x - 1]
        teN = self.error[pN]
        teNW = self.error[pNW]
        sumWN = teN + teW
        teNE = self.error[pNE]
        if props is not None:
            p = teW
            if abs(teN) > abs(p):
                p = teN
            if abs(teNW) > abs(p):
                p = teNW
            if abs(teNE) > abs(p):
                p = teNE
            props[WP_PROP] = p
        pr = self.prediction
        pr[0] = W + NE - N
        pr[1] = N - (((sumWN + teNE) * h.p1C) >> 5)
        pr[2] = W - (((sumWN + teNW) * h.p2C) >> 5)
        pr[3] = N - ((teNW * h.p3Ca + teN * h.p3Cb + teNE * h.p3Cc + (NN - N) * h.p3Cd + (NW - W) * h.p3Ce) >> 5)
        self.pred = self.weighted_average(pr, weights)
        if ((teN ^ teW) | (teN ^ teNW)) > 0:
            return (self.pred + 3) >> 3
        mx = max(W, NE, N)
        mn = min(W, NE, N)
        self.pred = max(mn, min(mx, self.pred))
        return (self.pred + 3) >> 3

    def update(self, val, x, y):
        xs = self.xs
        cur = 0 if y & 1 else xs + 2
        prev = xs + 2 if y & 1 else 0
        val <<= 3
        self.error[cur + x] = self.pred - val
        for i in range(4):
            err = (abs(self.prediction[i] - val) + 3) >> 3
            self.pred_errors[i][cur + x] = err
            self.pred_errors[i][prev + x + 1] += err


def predict_one(p, left, top, toptop, topleft, topright, leftleft, toprightright, wp):
    if p == 0:
        return 0
    if p == 1:
        return left
    if p == 2:
        return top
    if p == 3:
        return div2(left + top)
    if p == 4:      # Select
        pp = left + top - topleft
        return left if abs(pp - left) < abs(pp - top) else top
    if p == 5:
        return clamped_gradient(left, top, topleft)
    if p == 6:
        return wp
    if p == 7:
        return topright
    if p == 8:
        return topleft
    if p == 9:
        return leftleft
    if p == 10:
        return div2(left + topleft)
    if p == 11:
        return div2(topleft + top)
    if p == 12:
        return div2(top + topright)
    if p == 13:
        return cdiv(6 * top - 2 * toptop + 7 * left + leftleft + toprightright + 3 * topright + 8, 16)
    return 0


class Channel:
    def __init__(self, w, h, hshift=0, vshift=0):
        self.w, self.h, self.hshift, self.vshift = w, h, hshift, vshift
        self.d = [[0] * w for _ in range(h)]


class Tree:
    """MA tree: nodes (property, splitval, lchild, rchild, predictor, offset, multiplier)"""

    def __init__(self, nodes):
        self.nodes = nodes
        self.maxprop = max([n[0] for n in nodes] + [-1])
        self.uses_wp = any(n[0] == WP_PROP or (n[0] == -1 and n[4] == 6) for n in nodes)

    def lookup(self, props):
        i = 0
        nodes = self.nodes
        while True:
            n = nodes[i]
            if n[0] < 0:
                return n
            i = n[2] if props[n[0]] > n[1] else n[3]


def decode_tree(b, size_limit):
    code = decode_histograms(b, 6)
    r = Reader(code, b)
    nodes = []
    leaf = 0
    to_decode = 1
    while to_decode > 0:
        if len(nodes) > size_limit:
            raise JXLError("tree too large")
        to_decode -= 1
        prop1 = r.read(1)
        if prop1 > 256:
            raise JXLError("tree property")
        prop = prop1 - 1
        if prop == -1:
            pred = r.read(2)
            if pred >= 14:
                raise JXLError("predictor")
            off = unpack_signed(r.read(3))
            mlog = r.read(4)
            if mlog >= 31:
                raise JXLError("multiplier log")
            mbits = r.read(5)
            if mbits >= (1 << (31 - mlog)) - 1:
                raise JXLError("multiplier")
            nodes.append((-1, 0, leaf, 0, pred, off, (mbits + 1) << mlog))
            leaf += 1
            continue
        split = unpack_signed(r.read(0))
        n = len(nodes)
        nodes.append((prop, split, n + to_decode + 1, n + to_decode + 2, 0, 0, 1))
        to_decode += 2
    if not r.final_ok():
        raise JXLError("tree ANS state")
    return Tree(nodes)


class Transform:
    RCT, PALETTE, SQUEEZE = 0, 1, 2

    def read(self, b):
        self.id = b.u32(V(0), V(1), V(2), V(3))
        if self.id == 3:
            raise JXLError("invalid transform")
        self.begin_c = 0
        self.rct_type = 6
        self.num_c = 3
        self.nb_colors = 256
        self.nb_deltas = 0
        self.predictor = 0
        self.squeezes = []
        if self.id in (self.RCT, self.PALETTE):
            self.begin_c = b.u32(B(3), B(6, 8), B(10, 72), B(13, 1096))
        if self.id == self.RCT:
            self.rct_type = b.u32(V(6), B(2), B(4, 2), B(6, 10))
            if self.rct_type >= 42:
                raise JXLError("RCT type")
        if self.id == self.PALETTE:
            self.num_c = b.u32(V(1), V(3), V(4), B(13, 1))
            self.nb_colors = b.u32(B(8), B(10, 256), B(12, 1280), B(16, 5376))
            self.nb_deltas = b.u32(V(0), B(8, 1), B(10, 257), B(16, 1281))
            self.predictor = b.f(4)
            if self.predictor >= 14:
                raise JXLError("palette predictor")
        if self.id == self.SQUEEZE:
            n = b.u32(V(0), B(4, 1), B(6, 9), B(8, 41))
            for _ in range(n):
                hz = b.boolean()
                ip = b.boolean()
                bc = b.u32(B(3), B(6, 8), B(10, 72), B(13, 1096))
                nc = b.u32(V(1), V(2), V(3), B(4, 4))
                self.squeezes.append((hz, ip, bc, nc))
        return self


class ModImage:
    def __init__(self, w, h, bitdepth, channels):
        self.w, self.h, self.bitdepth = w, h, bitdepth
        self.ch = channels
        self.nb_meta = 0
        self.transforms = []

    # ---- meta application (channel list changes) ----
    def meta_apply(self, t):
        ch = self.ch
        if t.id == Transform.RCT:
            c1, c2 = t.begin_c, t.begin_c + 2
            self.check_equal(c1, c2)
        elif t.id == Transform.PALETTE:
            bc, ec = t.begin_c, t.begin_c + t.num_c - 1
            self.check_equal(bc, ec)
            nb = ec - bc + 1
            if bc >= self.nb_meta:
                self.nb_meta += 1
            else:
                if ec >= self.nb_meta:
                    raise JXLError("palette meta range")
                self.nb_meta += 2 - nb
            del ch[bc + 1:ec + 1]
            pc = Channel(t.nb_colors + t.nb_deltas, nb, -1, -1)
            ch.insert(0, pc)
        elif t.id == Transform.SQUEEZE:
            if not t.squeezes:
                t.squeezes = self.default_squeeze()
            for (hz, ip, bc, nc) in t.squeezes:
                ec = bc + nc - 1
                if bc < 0 or ec >= len(ch) or ec < bc:
                    raise JXLError("squeeze range")
                if bc < self.nb_meta:
                    if ec >= self.nb_meta or not ip:
                        raise JXLError("squeeze meta")
                    self.nb_meta += nc
                offset = ec + 1 if ip else len(ch)
                for c in range(bc, ec + 1):
                    w, h = ch[c].w, ch[c].h
                    if w == 0 or h == 0:
                        raise JXLError("squeezing empty channel")
                    if hz:
                        ch[c].w = (w + 1) // 2
                        if ch[c].hshift >= 0:
                            ch[c].hshift += 1
                        w = w - (w + 1) // 2
                    else:
                        ch[c].h = (h + 1) // 2
                        if ch[c].vshift >= 0:
                            ch[c].vshift += 1
                        h = h - (h + 1) // 2
                    ch[c].d = [[0] * ch[c].w for _ in range(ch[c].h)]
                    ph = Channel(w, h, ch[c].hshift, ch[c].vshift)
                    ch.insert(offset + (c - bc), ph)

    def check_equal(self, c1, c2):
        ch = self.ch
        if c1 > len(ch) or c2 >= len(ch) or c2 < c1:
            raise JXLError("transform channel range")
        if c1 < self.nb_meta <= c2:
            raise JXLError("transform meta mix")
        a = ch[c1]
        for c in range(c1 + 1, c2 + 1):
            o = ch[c]
            if (a.w, a.h, a.hshift, a.vshift) != (o.w, o.h, o.hshift, o.vshift):
                raise JXLError("transform channel mismatch")

    def default_squeeze(self):
        ch = self.ch
        nm = self.nb_meta
        nbc = len(ch) - nm
        w, h = ch[nm].w, ch[nm].h
        res = []
        if nbc > 2 and ch[nm + 1].w == w and ch[nm + 1].h == h:
            res.append((True, False, nm + 1, 2))
            res.append((False, False, nm + 1, 2))
        wide = w > h
        if not wide and h > 8:
            res.append((False, True, nm, nbc))
            h = (h + 1) // 2
        while w > 8 or h > 8:
            if w > 8:
                res.append((True, True, nm, nbc))
                w = (w + 1) // 2
            if h > 8:
                res.append((False, True, nm, nbc))
                h = (h + 1) // 2
        return res

    # ---- inverse transforms ----
    def undo_transforms(self, wp_hdr):
        while self.transforms:
            t = self.transforms.pop()
            if t.id == Transform.RCT:
                self.inv_rct(t.begin_c, t.rct_type)
            elif t.id == Transform.PALETTE:
                self.inv_palette(t, wp_hdr)
            else:
                self.inv_squeeze(t.squeezes)

    def inv_rct(self, m, rct_type):
        ch = self.ch
        self.check_equal(m, m + 2)
        if rct_type == 0:
            return
        perm = rct_type // 7
        custom = rct_type % 7
        c0, c1, c2 = ch[m], ch[m + 1], ch[m + 2]
        o0 = m + (perm % 3)
        o1 = m + ((perm + 1 + perm // 3) % 3)
        o2 = m + ((perm + 2 - perm // 3) % 3)
        if custom == 0:
            ch[o0], ch[o1], ch[o2] = c0, c1, c2
            return
        second, third = custom >> 1, custom & 1
        n0 = Channel(c0.w, c0.h, c0.hshift, c0.vshift)
        n1 = Channel(c0.w, c0.h, c0.hshift, c0.vshift)
        n2 = Channel(c0.w, c0.h, c0.hshift, c0.vshift)
        for y in range(c0.h):
            r0, r1, r2 = c0.d[y], c1.d[y], c2.d[y]
            q0, q1, q2 = n0.d[y], n1.d[y], n2.d[y]
            for x in range(c0.w):
                if custom == 6:
                    Y, Co, Cg = r0[x], r1[x], r2[x]
                    tmp = wrap32(Y - (Cg >> 1))
                    G = wrap32(Cg + tmp)
                    Bv = wrap32(tmp - (Co >> 1))
                    R = wrap32(Bv + Co)
                    q0[x], q1[x], q2[x] = R, G, Bv
                else:
                    F, S, T = r0[x], r1[x], r2[x]
                    if third:
                        T = wrap32(T + F)
                    if second == 1:
                        S = wrap32(S + F)
                    elif second == 2:
                        S = wrap32(S + (wrap32(F + T) >> 1))
                    q0[x], q1[x], q2[x] = F, S, T
        ch[o0], ch[o1], ch[o2] = n0, n1, n2

    def inv_palette(self, t, wp_hdr):
        ch = self.ch
        if self.nb_meta < 1:
            raise JXLError("palette without palette")
        nb = ch[0].h
        c0 = t.begin_c + 1
        if c0 >= len(ch):
            raise JXLError("palette channel")
        w, h = ch[c0].w, ch[c0].h
        for i in range(1, nb):
            ch.insert(c0 + 1, Channel(w, h, ch[c0].hshift, ch[c0].vshift))
        pal = ch[0]
        bd = min(self.bitdepth, 24)
        psize = pal.w
        if w == 0:
            pass
        elif t.nb_deltas == 0 and t.predictor == 0:
            idx_rows = [list(r) for r in ch[c0].d]
            for c in range(nb):
                out = ch[c0 + c]
                for y in range(h):
                    row = out.d[y]
                    ir = idx_rows[y]
                    for x in range(w):
                        index = ir[x]
                        if nb == 1:
                            index = max(0, min(psize - 1, index))
                        row[x] = palette_value(pal, index, c, psize, bd)
        else:
            idx_rows = [list(r) for r in ch[c0].d]
            for c in range(nb):
                out = ch[c0 + c]
                wp = WPState(wp_hdr, out.w) if t.predictor == 6 else None
                for y in range(h):
                    row = out.d[y]
                    for x in range(w):
                        index = idx_rows[y][x]
                        entry = palette_value(pal, index, c, psize, bd)
                        if index < t.nb_deltas:
                            g = self.predict_plain(out, x, y, t.predictor, wp)
                            v = g + entry
                        else:
                            v = entry
                        row[x] = v
                        if wp is not None:
                            wp.update(row[x], x, y)
        if c0 >= self.nb_meta:
            self.nb_meta -= 1
        else:
            self.nb_meta -= 2 - nb
        del ch[0]

    @staticmethod
    def predict_plain(chn, x, y, predictor, wp):
        d = chn.d
        w = chn.w
        row = d[y]
        prevr = d[y - 1] if y else None
        left = row[x - 1] if x else (prevr[x] if y else 0)
        top = prevr[x] if y else left
        topleft = prevr[x - 1] if (x and y) else left
        topright = prevr[x + 1] if (x + 1 < w and y) else top
        leftleft = row[x - 2] if x > 1 else left
        toptop = d[y - 2][x] if y > 1 else top
        trr = prevr[x + 2] if (x + 2 < w and y) else topright
        wpp = wp.predict(x, y, top, left, topright, topleft, toptop, None) if wp is not None else 0
        return predict_one(predictor, left, top, toptop, topleft, topright, leftleft, trr, wpp)

    def inv_squeeze(self, params):
        ch = self.ch
        for (hz, ip, bc, nc) in reversed(params):
            ec = bc + nc - 1
            if bc < 0 or ec >= len(ch):
                raise JXLError("squeeze range")
            offset = ec + 1 if ip else len(ch) + bc - ec - 1
            if bc < self.nb_meta:
                self.nb_meta -= nc
            for c in range(bc, ec + 1):
                rc = offset + c - bc
                if ch[c].w < ch[rc].w or ch[c].h < ch[rc].h:
                    raise JXLError("corrupted squeeze")
                ch[c] = unsqueeze(ch[c], ch[rc], hz)
            del ch[offset:offset + nc]


def smooth_tendency(B, a, n):
    diff = 0
    if B >= a >= n:
        diff = cdiv(4 * B - 3 * n - a + 6, 12)
        if diff - (diff & 1) > 2 * (B - a):
            diff = 2 * (B - a) + 1
        if diff + (diff & 1) > 2 * (a - n):
            diff = 2 * (a - n)
    elif B <= a <= n:
        diff = cdiv(4 * B - 3 * n - a - 6, 12)
        if diff + (diff & 1) < 2 * (B - a):
            diff = 2 * (B - a) - 1
        if diff - (diff & 1) < 2 * (a - n):
            diff = 2 * (a - n)
    return diff


def unsqueeze(avg_ch, res_ch, horizontal):
    if horizontal:
        if res_ch.w == 0:
            avg_ch.hshift -= 1
            return avg_ch
        out = Channel(avg_ch.w + res_ch.w, avg_ch.h, avg_ch.hshift - 1, avg_ch.vshift)
        if res_ch.h == 0:
            return out
        for y in range(avg_ch.h):
            pa, pr, po = avg_ch.d[y], res_ch.d[y], out.d[y]
            for x in range(res_ch.w):
                avg = pa[x]
                nxt = pa[x + 1] if x + 1 < avg_ch.w else avg
                left = po[2 * x - 1] if x else avg
                diff = pr[x] + smooth_tendency(left, avg, nxt)
                A = avg + div2(diff)
                po[2 * x] = A
                po[2 * x + 1] = A - diff
            if out.w & 1:
                po[out.w - 1] = pa[avg_ch.w - 1]
        return out
    if res_ch.h == 0:
        avg_ch.vshift -= 1
        return avg_ch
    out = Channel(avg_ch.w, avg_ch.h + res_ch.h, avg_ch.hshift, avg_ch.vshift - 1)
    if res_ch.w == 0:
        return out
    for y in range(res_ch.h):
        pa = avg_ch.d[y]
        pn = avg_ch.d[y + 1 if y + 1 < avg_ch.h else y]
        pr = res_ch.d[y]
        po = out.d[2 * y]
        pno = out.d[2 * y + 1]
        pp = out.d[2 * y - 1] if y > 0 else pa
        for x in range(avg_ch.w):
            avg = pa[x]
            diff = pr[x] + smooth_tendency(pp[x], avg, pn[x])
            o = avg + div2(diff)
            po[x] = o
            pno[x] = o - diff
    if out.h & 1:
        out.d[out.h - 1] = list(avg_ch.d[avg_ch.h - 1])
    return out


DELTA_PALETTE = [
    (0, 0, 0), (4, 4, 4), (11, 0, 0), (0, 0, -13), (0, -12, 0), (-10, -10, -10), (-18, -18, -18), (-27, -27, -27),
    (-18, -18, 0), (0, 0, -32), (-32, 0, 0), (-37, -37, -37), (0, -32, -32), (24, 24, 45), (50, 50, 50),
    (-45, -24, -24), (-24, -45, -45), (0, -24, -24), (-34, -34, 0), (-24, 0, -24), (-45, -45, -24), (64, 64, 64),
    (-32, 0, -32), (0, -32, 0), (-32, 0, 32), (-24, -45, -24), (45, 24, 45), (24, -24, -45), (-45, -24, 24),
    (80, 80, 80), (64, 0, 0), (0, 0, -64), (0, -64, -64), (-24, -24, 45), (96, 96, 96), (64, 64, 0),
    (45, -24, -24), (34, -34, 0), (112, 112, 112), (24, -45, -45), (45, 45, -24), (0, -32, 32), (24, -24, 45),
    (0, 96, 96), (45, -24, 24), (24, -45, -24), (-24, -45, 24), (0, -64, 0), (96, 0, 0), (128, 128, 128),
    (64, 0, 64), (144, 144, 144), (96, 96, 0), (-36, -36, 36), (45, -24, -45), (45, -45, -24), (0, 0, -96),
    (0, 128, 128), (0, 96, 0), (45, 24, -45), (-128, 0, 0), (24, -45, 24), (-45, 24, -45), (64, 0, -64),
    (64, -64, -64), (96, 0, 96), (45, -45, 24), (24, 45, -45), (64, 64, -64), (128, 128, 0), (0, 0, -128),
    (-24, 45, -45)]


def palette_value(pal, index, c, psize, bd):
    if index < 0:
        if c >= 3:
            return 0
        index = -(index + 1)
        index %= 1 + 2 * (len(DELTA_PALETTE) - 1)
        r = DELTA_PALETTE[(index + 1) >> 1][c] * (1 if index & 1 else -1)
        if bd > 8:
            r *= 1 << (bd - 8)
        return r
    if psize <= index < psize + 64:
        if c >= 3:
            return 0
        index -= psize
        index >>= c * 2
        return (((index % 4) * ((1 << bd) - 1)) >> 2) + (1 << max(0, bd - 3))
    if psize + 64 <= index:
        if c >= 3:
            return 0
        index -= psize + 64
        if c == 1:
            index //= 5
        elif c == 2:
            index //= 25
        return ((index % 5) * ((1 << bd) - 1)) >> 2
    return pal.d[c][index]


def modular_decode(b, img, stream_id, max_chan_size, gtree, gcode, undo):
    """ModularGenericDecompress: group header, transforms (meta), channels; undo transforms if asked.
    Returns the group header."""
    if not img.ch:
        return None
    use_global = b.boolean()
    wp = WPHeader().read(b)
    nt = b.u32(V(0), V(1), B(4, 2), B(8, 18))
    tr = [Transform().read(b) for _ in range(nt)]
    img.transforms = tr
    for t in tr:
        img.meta_apply(t)
    nch = len(img.ch)
    dist_mult = 0
    any_ch = False
    for i in range(nch):
        c = img.ch[i]
        if not c.w or not c.h:
            continue
        if i >= img.nb_meta and (c.w > max_chan_size or c.h > max_chan_size):
            break
        dist_mult = max(dist_mult, c.w)
        any_ch = True
    if any_ch:
        if not use_global:
            limit = 1024
            for i in range(nch):
                c = img.ch[i]
                if i >= img.nb_meta and (c.w > max_chan_size or c.h > max_chan_size):
                    break
                limit += c.w * c.h
            tree = decode_tree(b, min(1 << 20, limit))
            code = decode_histograms(b, (len(tree.nodes) + 1) // 2)
        else:
            if gtree is None:
                raise JXLError("no global tree")
            tree, code = gtree, gcode
        r = Reader(code, b, dist_mult)
        for i in range(nch):
            c = img.ch[i]
            if not c.w or not c.h:
                continue
            if i >= img.nb_meta and (c.w > max_chan_size or c.h > max_chan_size):
                break
            decode_channel(r, tree, wp, img, i, stream_id)
        if not r.final_ok():
            raise JXLError("modular ANS state")
    if undo:
        img.undo_transforms(wp)

    class H:
        pass
    h = H()
    h.wp, h.use_global = wp, use_global
    return h


def decode_channel(r, tree, wp_hdr, img, chan, group_id):
    ch = img.ch[chan]
    w, h = ch.w, ch.h
    nprops = NUM_NONREF_PROPS
    if tree.maxprop + 1 > NUM_NONREF_PROPS:
        nprops = -(-(tree.maxprop + 1 - NUM_NONREF_PROPS) // 4) * 4 + NUM_NONREF_PROPS
    nref = nprops - NUM_NONREF_PROPS
    props = [0] * nprops
    props[0] = chan
    props[1] = group_id
    wp = WPState(wp_hdr, w) if tree.uses_wp else None
    refchans = []
    if nref:
        for j in range(chan - 1, -1, -1):
            if len(refchans) * 4 >= nref:
                break
            o = img.ch[j]
            if (o.w, o.h, o.hshift, o.vshift) != (ch.w, ch.h, ch.hshift, ch.vshift):
                continue
            refchans.append(o)
    d = ch.d
    for y in range(h):
        row = d[y]
        prevr = d[y - 1] if y else None
        pp2 = d[y - 2] if y > 1 else None
        props[2] = y
        props[9] = 0
        for x in range(w):
            left = row[x - 1] if x else (prevr[x] if y else 0)
            top = prevr[x] if y else left
            topleft = prevr[x - 1] if (x and y) else left
            topright = prevr[x + 1] if (x + 1 < w and y) else top
            leftleft = row[x - 2] if x > 1 else left
            toptop = pp2[x] if y > 1 else top
            trr = prevr[x + 2] if (x + 2 < w and y) else topright
            props[3] = x
            props[4] = abs(top)
            props[5] = abs(left)
            props[6] = top
            props[7] = left
            props[8] = left - props[9]
            props[9] = left + top - topleft
            props[10] = left - topleft
            props[11] = topleft - top
            props[12] = top - topright
            props[13] = top - toptop
            props[14] = left - leftleft
            wpp = 0
            if wp is not None:
                wpp = wp.predict(x, y, top, left, topright, topleft, toptop, props)
            if nref:
                k = NUM_NONREF_PROPS
                for o in refchans:
                    od = o.d
                    v = od[y][x]
                    vleft = od[y][x - 1] if x else 0
                    vtop = od[y - 1][x] if y else vleft
                    vtl = od[y - 1][x - 1] if (x and y) else vleft
                    pv = clamped_gradient(vleft, vtop, vtl)
                    props[k] = abs(v)
                    props[k + 1] = v
                    props[k + 2] = abs(v - pv)
                    props[k + 3] = v - pv
                    k += 4
                while k < nprops:
                    props[k] = 0
                    k += 1
            leaf = tree.lookup(props)
            guess = leaf[5] + predict_one(leaf[4], left, top, toptop, topleft, topright, leftleft, trr, wpp)
            v = r.read(leaf[2])
            row[x] = wrap32(unpack_signed(v) * leaf[6] + guess)
            if wp is not None:
                wp.update(row[x], x, y)


# ====================================================================== VarDCT: tables

# All VarDCT and rendering arithmetic is float32 (numpy) without fused multiply-add, in the
# operation order of libjxl's scalar code (MulAdd( a, b, c ) = a * b + c, rounded twice).  libjxl
# itself uses FMA on AVX2 and rcpps in AdjustQuantBias, so it agrees within a small tolerance only;
# the assembly decoder follows this model exactly.

F32 = np.float32
SQRT2 = F32(1.41421356237)
ALMOST_ZERO = 1e-8
INV_SIGMA_NUM = F32(-1.1715728752538099024)
MIN_SIGMA = F32(-3.90524291751269967465540850526868)

# AC strategies (AcStrategyType); name ROWSxCOLS in pixels
(ACS_DCT, ACS_IDENTITY, ACS_DCT2X2, ACS_DCT4X4, ACS_DCT16X16, ACS_DCT32X32, ACS_DCT16X8, ACS_DCT8X16,
 ACS_DCT32X8, ACS_DCT8X32, ACS_DCT32X16, ACS_DCT16X32, ACS_DCT4X8, ACS_DCT8X4, ACS_AFV0, ACS_AFV1, ACS_AFV2,
 ACS_AFV3) = range(18)
NUM_ACS = 27
ACS_CX = [1, 1, 1, 1, 2, 4, 1, 2, 1, 4, 2, 4, 1, 1, 1, 1, 1, 1, 8, 4, 8, 16, 8, 16, 32, 16, 32]
ACS_CY = [1, 1, 1, 1, 2, 4, 2, 1, 4, 1, 4, 2, 1, 1, 1, 1, 1, 1, 8, 8, 4, 16, 16, 8, 32, 32, 16]
ACS_LOG2 = [0, 0, 0, 0, 2, 4, 1, 1, 2, 2, 3, 3, 0, 0, 0, 0, 0, 0, 6, 5, 5, 8, 7, 7, 10, 9, 9]
STRATEGY_ORDER = [0, 1, 1, 1, 2, 3, 4, 4, 5, 5, 6, 6, 1, 1, 1, 1, 1, 1, 7, 8, 8, 9, 10, 10, 11, 12, 12]
ACS_QTABLE = [0, 1, 2, 3, 4, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 10, 10, 11, 12, 12, 13, 14, 14, 15, 16, 16]
QT_SIZE_X = [1, 1, 1, 1, 2, 4, 1, 1, 2, 1, 1, 8, 4, 16, 8, 32, 16]
QT_SIZE_Y = [1, 1, 1, 1, 2, 4, 2, 4, 4, 1, 1, 8, 8, 16, 16, 32, 32]
NUM_ORDERS = 13
GROUP_BLOCKS = 32           # VarDCT groups are always 256 x 256 pixels

NONZERO_BUCKETS = 37
ZERO_DENSITY_CONTEXTS = 458
COEFF_FREQ_CTX = [
    0xBAD, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22,
    23, 23, 23, 23, 24, 24, 24, 24, 25, 25, 25, 25, 26, 26, 26, 26,
    27, 27, 27, 27, 28, 28, 28, 28, 29, 29, 29, 29, 30, 30, 30, 30]
COEFF_NZ_CTX = [
    0xBAD, 0, 31, 62, 62, 93, 93, 93, 93, 123, 123, 123, 123,
    152, 152, 152, 152, 152, 152, 152, 152, 180, 180, 180, 180, 180,
    180, 180, 180, 180, 180, 180, 180, 206, 206, 206, 206, 206, 206,
    206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206,
    206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206]
DEFAULT_CTX_MAP = [
    0, 1, 2, 2, 3, 3, 4, 5, 6, 6, 6, 6, 6,
    7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 14, 14, 14,
    7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 14, 14, 14]

AFV_FREQS = [0xBAD, 0xBAD, 0.8517778890324296, 5.37778436506804, 0xBAD, 0xBAD, 4.734747904497923,
             5.449245381693219, 1.6598270267479331, 4, 7.275749096817861, 10.423227632456525,
             2.662932286148962, 7.630657783650829, 8.962388608184032, 12.97166202570235]

# DequantMatricesLibraryDef (weights are final, the bitstream ones get multiplied by 64)
_TAIL_A = [-1.025, -0.78, -0.65012, -0.19041574084286472, -0.20819395464, -0.421064, -0.32733845535848671]
_TAIL_B = [-0.3041958212306401, -0.3633036457487539, -0.35660379990111464, -0.3443074455424403,
           -0.33699592683512467, -0.30180866526242109, -0.27321683125358037]
_TAIL_C = [-1.2, -1.2, -0.8, -0.7, -0.7, -0.4, -0.5]


def _big(f, a, b, c):
    return ('dct', ([[f * a] + _TAIL_A, [f * b] + _TAIL_B, [f * c] + _TAIL_C], 8))


_DCT4X8_PARAMS = ([[2198.050556016380522, -0.96269623020744692, -0.76194253026666783, -0.6551140670773547],
                   [764.3655248643528689, -0.92630200888366945, -0.9675229603596517, -0.27845290869168118],
                   [527.107573587542228, -1.4594385811273854, -1.450082094097871593, -1.5843722511996204]], 4)
_DCT4X4_PARAMS = ([[2200.0, 0.0, 0.0, 0.0], [392.0, 0.0, 0.0, 0.0], [112.0, -0.25, -0.25, -0.5]], 4)
QUANT_LIBRARY = [
    ('dct', ([[3150.0, 0.0, -0.4, -0.4, -0.4, -2.0], [560.0, 0.0, -0.3, -0.3, -0.3, -0.3],
              [512.0, -2.0, -1.0, 0.0, -1.0, -2.0]], 6)),
    ('id', [[280.0, 3160.0, 3160.0], [60.0, 864.0, 864.0], [18.0, 200.0, 200.0]]),
    ('dct2', [[3840.0, 2560.0, 1280.0, 640.0, 480.0, 300.0], [960.0, 640.0, 320.0, 180.0, 140.0, 120.0],
              [640.0, 320.0, 128.0, 64.0, 32.0, 16.0]]),
    ('dct4', _DCT4X4_PARAMS, [[1.0, 1.0]] * 3),
    ('dct', ([[8996.8725711814115328, -1.3000777393353804, -0.49424529824571225, -0.439093774457103443,
               -0.6350101832695744, -0.90177264050827612, -1.6162099239887414],
              [3191.48366296844234752, -0.67424582104194355, -0.80745813428471001, -0.44925837484843441,
               -0.35865440981033403, -0.31322389111877305, -0.37615025315725483],
              [1157.50408145487200256, -2.0531423165804414, -1.4, -0.50687130033378396,
               -0.42708730624733904, -1.4856834539296244, -4.9209142884401604]], 7)),
    ('dct', ([[15718.40830982518931456, -1.025, -0.98, -0.9012, -0.4, -0.48819395464, -0.421064, -0.27],
              [7305.7636810695983104, -0.8041958212306401, -0.7633036457487539, -0.55660379990111464,
               -0.49785304658857626, -0.43699592683512467, -0.40180866526242109, -0.27321683125358037],
              [3803.53173721215041536, -3.060733579805728, -2.0413270132490346, -2.0235650159727417,
               -0.5495389509954993, -0.4, -0.4, -0.3]], 8)),
    ('dct', ([[7240.7734393502, -0.7, -0.7, -0.2, -0.2, -0.2, -0.5],
              [1448.15468787004, -0.5, -0.5, -0.5, -0.2, -0.2, -0.2],
              [506.854140754517, -1.4, -0.2, -0.5, -0.5, -1.5, -3.6]], 7)),
    ('dct', ([[16283.2494710648897, -1.7812845336559429, -1.6309059012653515, -1.0382179034313539, -0.85, -0.7,
               -0.9, -1.2360638576849587],
              [5089.15750884921511936, -0.320049391452786891, -0.35362849922161446, -0.30340000000000003, -0.61,
               -0.5, -0.5, -0.6],
              [3397.77603275308720128, -0.321327362693153371, -0.34507619223117997, -0.70340000000000003, -0.9,
               -1.0, -1.0, -1.1754605576265209]], 8)),
    ('dct', ([[13844.97076442300573, -0.97113799999999995, -0.658, -0.42026, -0.22712, -0.2206, -0.226, -0.6],
              [4798.964084220744293, -0.61125308982767057, -0.83770786552491361, -0.79014862079498627,
               -0.2692727459704829, -0.38272769465388551, -0.22924222653091453, -0.20719098826199578],
              [1807.236946760964614, -1.2, -1.2, -0.7, -0.7, -0.7, -0.4, -0.5]], 8)),
    ('dct4x8', _DCT4X8_PARAMS, [1.0, 1.0, 1.0]),
    ('afv', [[3072.0, 3072.0, 256.0, 256.0, 256.0, 414.0, 0.0, 0.0, 0.0],
             [1024.0, 1024.0, 50.0, 50.0, 50.0, 58.0, 0.0, 0.0, 0.0],
             [384.0, 384.0, 12.0, 12.0, 12.0, 22.0, -0.25, -0.25, -0.25]], _DCT4X8_PARAMS, _DCT4X4_PARAMS),
    _big(0.9, 26629.073922049845, 9311.3238710010046, 4992.2486445538634),
    _big(0.65, 23629.073922049845, 8611.3238710010046, 4492.2486445538634),
    _big(1.8, 26629.073922049845, 9311.3238710010046, 4992.2486445538634),
    _big(1.3, 23629.073922049845, 8611.3238710010046, 4492.2486445538634),
    _big(3.6, 26629.073922049845, 9311.3238710010046, 4992.2486445538634),
    _big(2.6, 23629.073922049845, 8611.3238710010046, 4492.2486445538634),
]

RESAMPLE_SCALES = {
    (8, 1): [1.000000000000000000],
    (16, 2): [1.000000000000000000, 0.901764195028874394],
    (32, 4): [1.000000000000000000, 0.974886821136879522, 0.901764195028874394, 0.787054918159101335],
    (64, 8): [1.0000000000000000, 0.9936866130906366, 0.9748868211368796, 0.9440180941651672, 0.9017641950288744, 0.8490574973847023, 0.7870549181591013, 0.7171081282466044],
    (128, 16): [1.0, 0.9984194528776054, 0.9936866130906366, 0.9858278282666936, 0.9748868211368796, 0.9609244059440204, 0.9440180941651672, 0.9242615922757944, 0.9017641950288744, 0.8766500784429904, 0.8490574973847023, 0.8191378932865928, 0.7870549181591013, 0.7529833816270532, 0.7171081282466044, 0.6796228528314651],
    (256, 32): [1.0, 0.9996047255830407, 0.9984194528776054, 0.9964458326264695, 0.9936866130906366, 0.9901456355893141, 0.9858278282666936, 0.9807391980963174, 0.9748868211368796, 0.9682788310563117, 0.9609244059440204, 0.9528337534340876, 0.9440180941651672, 0.9344896436056892, 0.9242615922757944, 0.913348084400198, 0.9017641950288744, 0.8895259056651056, 0.8766500784429904, 0.8631544288990163, 0.8490574973847023, 0.8343786191696513, 0.8191378932865928, 0.8033561501721485, 0.7870549181591013, 0.7702563888779096, 0.7529833816270532, 0.7352593067735488, 0.7171081282466044, 0.6985543251889097, 0.6796228528314651, 0.6603391026591464],
    (1, 8): [1.000000000000000000],
    (2, 16): [1.000000000000000000, 1.108937353592731823],
    (4, 32): [1.000000000000000000, 1.025760096781116015, 1.108937353592731823, 1.270559368765487251],
    (8, 64): [1.0000000000000000, 1.0063534990068217, 1.0257600967811158, 1.0593017296817173, 1.1089373535927318, 1.1777765381970435, 1.2705593687654873, 1.3944898413647777],
    (16, 128): [1.0, 1.0015830492062623, 1.0063534990068217, 1.0143759095928793, 1.0257600967811158, 1.0406645869480142, 1.0593017296817173, 1.0819447744633812, 1.1089373535927318, 1.1407059950032632, 1.1777765381970435, 1.2207956782315876, 1.2705593687654873, 1.3280505578213306, 1.3944898413647777, 1.4714043176061107],
    (32, 256): [1.0, 1.0003954307206069, 1.0015830492062623, 1.0035668445360069, 1.0063534990068217, 1.009952439375063, 1.0143759095928793, 1.0196390660647288, 1.0257600967811158, 1.0327603660498115, 1.0406645869480142, 1.049501024072585, 1.0593017296817173, 1.0701028169146336, 1.0819447744633812, 1.0948728278734026, 1.1089373535927318, 1.124194353004584, 1.1407059950032632, 1.158541237256391, 1.1777765381970435, 1.1984966740820495, 1.2207956782315876, 1.244777922949508, 1.2705593687654873, 1.2982690107339132, 1.3280505578213306, 1.3600643892400104, 1.3944898413647777, 1.4315278911623237, 1.4714043176061107, 1.5143734423314616],
}
AFV_BASIS = [
    [0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.25],
    [0.876902929799142, 0.2206518106944235, -0.10140050393753763, -0.1014005039375375, 0.2206518106944236, -0.10140050393753777, -0.10140050393753772, -0.10140050393753763, -0.10140050393753758, -0.10140050393753769, -0.1014005039375375, -0.10140050393753768, -0.10140050393753768, -0.10140050393753759, -0.10140050393753763, -0.10140050393753741],
    [0.0, 0.0, 0.40670075830260755, 0.44444816619734445, 0.0, 0.0, 0.19574399372042936, 0.2929100136981264, -0.40670075830260716, -0.19574399372042872, 0.0, 0.11379074460448091, -0.44444816619734384, -0.29291001369812636, -0.1137907446044814, 0.0],
    [0.0, 0.0, -0.21255748058288748, 0.3085497062849767, 0.0, 0.4706702258572536, -0.1621205195722993, 0.0, -0.21255748058287047, -0.16212051957228327, -0.47067022585725277, -0.1464291867126764, 0.3085497062849487, 0.0, -0.14642918671266536, 0.4251149611657548],
    [0.0, -0.7071067811865474, 0.0, 0.0, 0.7071067811865476, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [-0.4105377591765233, 0.6235485373547691, -0.06435071657946274, -0.06435071657946266, 0.6235485373547694, -0.06435071657946284, -0.0643507165794628, -0.06435071657946274, -0.06435071657946272, -0.06435071657946279, -0.06435071657946266, -0.06435071657946277, -0.06435071657946277, -0.06435071657946273, -0.06435071657946274, -0.0643507165794626],
    [0.0, 0.0, -0.4517556589999482, 0.15854503551840063, 0.0, -0.04038515160822202, 0.0074182263792423875, 0.39351034269210167, -0.45175565899994635, 0.007418226379244351, 0.1107416575309343, 0.08298163094882051, 0.15854503551839705, 0.3935103426921022, 0.0829816309488214, -0.45175565899994796],
    [0.0, 0.0, -0.304684750724869, 0.5112616136591823, 0.0, 0.0, -0.290480129728998, -0.06578701549142804, 0.304684750724884, 0.2904801297290076, 0.0, -0.23889773523344604, -0.5112616136592012, 0.06578701549142545, 0.23889773523345467, 0.0],
    [0.0, 0.0, 0.3017929516615495, 0.25792362796341184, 0.0, 0.16272340142866204, 0.09520022653475037, 0.0, 0.3017929516615503, 0.09520022653475055, -0.16272340142866173, -0.35312385449816297, 0.25792362796341295, 0.0, -0.3531238544981624, -0.6035859033230976],
    [0.0, 0.0, 0.40824829046386274, 0.0, 0.0, 0.0, 0.0, -0.4082482904638628, -0.4082482904638635, 0.0, 0.0, -0.40824829046386296, 0.0, 0.4082482904638634, 0.408248290463863, 0.0],
    [0.0, 0.0, 0.1747866975480809, 0.0812611176717539, 0.0, 0.0, -0.3675398009862027, -0.307882213957909, -0.17478669754808135, 0.3675398009862011, 0.0, 0.4826689115059883, -0.08126111767175039, 0.30788221395790305, -0.48266891150598584, 0.0],
    [0.0, 0.0, -0.21105601049335784, 0.18567180916109802, 0.0, 0.0, 0.49215859013738733, -0.38525013709251915, 0.21105601049335806, -0.49215859013738905, 0.0, 0.17419412659916217, -0.18567180916109904, 0.3852501370925211, -0.1741941265991621, 0.0],
    [0.0, 0.0, -0.14266084808807264, -0.3416446842253372, 0.0, 0.7367497537172237, 0.24627107722075148, -0.08574019035519306, -0.14266084808807344, 0.24627107722075137, 0.14883399227113567, -0.04768680350229251, -0.3416446842253373, -0.08574019035519267, -0.047686803502292804, -0.14266084808807242],
    [0.0, 0.0, -0.13813540350758585, 0.3302282550303788, 0.0, 0.08755115000587084, -0.07946706605909573, -0.4613374887461511, -0.13813540350758294, -0.07946706605910261, 0.49724647109535086, 0.12538059448563663, 0.3302282550303805, -0.4613374887461554, 0.12538059448564315, -0.13813540350758452],
    [0.0, 0.0, -0.17437602599651067, 0.0702790691196284, 0.0, -0.2921026642334881, 0.3623817333531167, 0.0, -0.1743760259965108, 0.36238173335311646, 0.29210266423348785, -0.4326608024727445, 0.07027906911962818, 0.0, -0.4326608024727457, 0.34875205199302267],
    [0.0, 0.0, 0.11354987314994337, -0.07417504595810355, 0.0, 0.19402893032594343, -0.435190496523228, 0.21918684838857466, 0.11354987314994257, -0.4351904965232251, 0.5550443808910661, -0.25468277124066463, -0.07417504595810233, 0.2191868483885728, -0.25468277124066413, 0.1135498731499429],
]
UP4_W = [
    -0.02419067, -0.03491987, -0.03693351, -0.03094285, -0.00529785, -0.01663432, -0.03556863, -0.03888905,
    -0.03516850, -0.00989469, 0.23651958, 0.33392945, -0.01073543, -0.01313181, -0.03556694, 0.13048175,
    0.40103025, 0.03951150, -0.02077584, 0.46914198, -0.00209270, -0.01484589, -0.04064806, 0.18942530,
    0.56279892, 0.06674400, -0.02335494, -0.03551682, -0.00754830, -0.02267919, -0.02363578, 0.00315804,
    -0.03399098, -0.01359519, -0.00091653, -0.00335467, -0.01163294, -0.01610294, -0.00974088, -0.00191622,
    -0.01095446, -0.03198464, -0.04455121, -0.02799790, -0.00645912, 0.06390599, 0.22963888, 0.00630981,
    -0.01897349, 0.67537268, 0.08483369, -0.02534994, -0.02205197, -0.01667999, -0.00384443,
]
UP8_W = [
    -0.02928613, -0.03706353, -0.03783812, -0.03324558, -0.00447632, -0.02519406, -0.03752601, -0.03901508,
    -0.03663285, -0.00646649, -0.02066407, -0.03838633, -0.04002101, -0.03900035, -0.00901973, -0.01626393,
    -0.03954148, -0.04046620, -0.03979621, -0.01224485, 0.29895328, 0.35757708, -0.02447552, -0.01081748,
    -0.04314594, 0.23903219, 0.41119301, -0.00573046, -0.01450239, -0.04246845, 0.17567618, 0.45220643,
    0.02287757, -0.01936783, -0.03583255, 0.11572472, 0.47416733, 0.06284440, -0.02685066, 0.42720050,
    -0.02248939, -0.01155273, -0.04562755, 0.28689496, 0.49093869, -0.00007891, -0.01545926, -0.04562659,
    0.21238920, 0.53980934, 0.03369474, -0.02070211, -0.03866988, 0.14229550, 0.56593398, 0.08045181,
    -0.02888298, -0.03680918, -0.00542229, -0.02920477, -0.02788574, -0.02118180, -0.03942402, -0.00775547,
    -0.02433614, -0.03193943, -0.02030828, -0.04044014, -0.01074016, -0.01930822, -0.03620399, -0.01974125,
    -0.03919545, -0.01456093, -0.00045072, -0.00360110, -0.01020207, -0.01231907, -0.00638988, -0.00071592,
    -0.00279122, -0.00957115, -0.01288327, -0.00730937, -0.00107783, -0.00210156, -0.00890705, -0.01317668,
    -0.00813895, -0.00153491, -0.02128481, -0.04173044, -0.04831487, -0.03293190, -0.00525260, -0.01720322,
    -0.04052736, -0.05045706, -0.03607317, -0.00738030, -0.01341764, -0.03965629, -0.05151616, -0.03814886,
    -0.01005819, 0.18968273, 0.33063684, -0.01300105, -0.01372950, -0.04017465, 0.13727832, 0.36402234,
    0.01027890, -0.01832107, -0.03365072, 0.08734506, 0.38194295, 0.04338228, -0.02525993, 0.56408126,
    0.00458352, -0.01648227, -0.04887868, 0.24585519, 0.62026135, 0.04314807, -0.02213737, -0.04158014,
    0.16637289, 0.65027023, 0.09621636, -0.03101388, -0.04082742, -0.00904519, -0.02790922, -0.02117818,
    0.00798662, -0.03995711, -0.01243427, -0.02231705, -0.02946266, 0.00992055, -0.03600283, -0.01684920,
    -0.00111684, -0.00411204, -0.01297130, -0.01723725, -0.01022545, -0.00165306, -0.00313110, -0.01218016,
    -0.01763266, -0.01125620, -0.00231663, -0.01374149, -0.03797620, -0.05142937, -0.03117307, -0.00581914,
    -0.01064003, -0.03608089, -0.05272168, -0.03375670, -0.00795586, 0.09628104, 0.27129991, -0.00353779,
    -0.01734151, -0.03153981, 0.05686230, 0.28500998, 0.02230594, -0.02374955, 0.68214326, 0.05018048,
    -0.02320852, -0.04383616, 0.18459474, 0.71517975, 0.10805613, -0.03263677, -0.03637639, -0.01394373,
    -0.02511203, -0.01728636, 0.05407331, -0.02867568, -0.01893131, -0.00240854, -0.00446511, -0.01636187,
    -0.02377053, -0.01522848, -0.00333334, -0.00819975, -0.02964169, -0.04499287, -0.02745350, -0.00612408,
    0.02727416, 0.19446600, 0.00159832, -0.02232473, 0.74982506, 0.11452620, -0.03348048, -0.01605681,
    -0.02070339, -0.00458223,
]

RESAMPLE_F32 = {k: np.array(v, F32) for k, v in RESAMPLE_SCALES.items()}
AFV_BASIS_F32 = np.array(AFV_BASIS, F32)

# ====================================================================== VarDCT: float helpers


def fast_log2f(x):
    """FastLog2f: (2,2) rational polynomial of log1p after range reduction to [-1/3, 1/3]"""
    x = np.asarray(x, F32)
    xb = x.view(np.int32)
    e = (xb - np.int32(0x3f2aaaab)) >> 23
    t = (xb - (e << 23)).view(F32) - F32(1)
    yp = (F32(7.4245873327820566E-01) * t + F32(1.4287160470083755E+00)) * t + F32(-1.8503833400518310E-06)
    yq = (F32(1.7409343003366853E-01) * t + F32(1.0096718572241148E+00)) * t + F32(9.9032814277590719E-01)
    return yp / yq + e.astype(F32)


def fast_pow2f(x):
    x = np.asarray(x, F32)
    fl = np.floor(x)
    ex = ((fl.astype(np.int32) + np.int32(127)) << 23).view(F32)
    fr = x - fl
    num = fr + F32(1.01749063e+01)
    num = num * fr + F32(4.88687798e+01)
    num = num * fr + F32(9.85506591e+01)
    num = num * ex
    den = fr * F32(2.10242958e-01) + F32(-2.22328856e-02)
    den = den * fr + F32(-1.94414990e+01)
    den = den * fr + F32(9.85506633e+01)
    return num / den


def fast_powf(b, e):
    return fast_pow2f(fast_log2f(b) * np.asarray(e, F32))


# ====================================================================== VarDCT: DCT

_WC = {}


def wc_mult(n, ndim):
    """WcMultipliers<n> = 0.5 / cos( (i + 0.5) * pi / n ), shaped to broadcast over axis 0"""
    m = _WC.get(n)
    if m is None:
        m = np.array([0.5 / math.cos((i + 0.5) * math.pi / n) for i in range(n // 2)], F32)
        _WC[n] = m
    return m.reshape((n // 2,) + (1,) * (ndim - 1))


def idct1d(a):
    """IDCT1DImpl along axis 0 (any trailing shape)"""
    n = a.shape[0]
    if n == 1:
        return a.copy()
    if n == 2:
        return np.stack([a[0] + a[1], a[0] - a[1]])
    h = n // 2
    even = idct1d(a[0::2])
    odd = a[1::2]
    bt = np.empty_like(odd)                     # BTranspose
    bt[1:] = odd[1:] + odd[:-1]
    bt[0] = odd[0] * SQRT2
    t = wc_mult(n, a.ndim) * idct1d(bt)
    out = np.empty_like(a)
    out[:h] = t + even                          # MulAdd( mul, in2, in1 )
    out[h:] = (even - t)[::-1]                  # NegMulAdd( mul, in2, in1 )
    return out


def dct1d_impl(m):
    n = m.shape[0]
    if n == 1:
        return m.copy()
    if n == 2:
        return np.stack([m[0] + m[1], m[0] - m[1]])
    h = n // 2
    rev = m[h:][::-1]
    t0 = dct1d_impl(m[:h] + rev)                # AddReverse
    t1 = dct1d_impl((m[:h] - rev) * wc_mult(n, m.ndim))
    b = np.empty_like(t1)                       # B
    b[0] = t1[0] * SQRT2 + t1[1]
    b[1:-1] = t1[1:-1] + t1[2:]
    b[-1] = t1[-1]
    out = np.empty_like(m)
    out[0::2] = t0
    out[1::2] = b
    return out


def fdct1d(m):
    return dct1d_impl(m) * F32(1.0 / m.shape[0])


def idct2d(c, rows, cols):
    """ComputeScaledIDCT< rows, cols >: c (K, rows * cols) coefficients -> (K, rows, cols) pixels"""
    k = c.shape[0]
    if rows < cols:
        d = c.reshape(k, rows, cols).transpose(2, 0, 1)
    else:
        d = c.reshape(k, cols, rows).transpose(1, 0, 2)
    t = idct1d(d).transpose(2, 1, 0)
    return idct1d(t).transpose(1, 0, 2)


def dct2d(p, rows, cols):
    """ComputeScaledDCT< rows, cols > of one (rows, cols) patch; result in coefficient layout (min, max)"""
    r = fdct1d(fdct1d(p).T)
    return r.T if rows < cols else r


def llf_from_dc(acs, patch):
    """LowestFrequenciesFromDC (ReinterpretingDCT) for a multi-block strategy: (min, max) values"""
    cy, cx = ACS_CY[acs], ACS_CX[acs]
    r = dct2d(patch, cy, cx)
    if cy < cx:
        sa, sb = RESAMPLE_F32[(cy, 8 * cy)], RESAMPLE_F32[(cx, 8 * cx)]
    else:
        sa, sb = RESAMPLE_F32[(cx, 8 * cx)], RESAMPLE_F32[(cy, 8 * cy)]
    return (r * sa[:, None]) * sb[None, :]


def _dcs4(c):
    b00, b01, b10, b11 = c[:, 0, 0], c[:, 0, 1], c[:, 1, 0], c[:, 1, 1]
    return [((b00 + b01) + b10) + b11, ((b00 + b01) - b10) - b11,
            ((b00 - b01) + b10) - b11, ((b00 - b01) - b10) + b11]


def _idct2top(blk, s):
    n = s // 2
    c00, c01 = blk[:, :n, :n], blk[:, :n, n:s]
    c10, c11 = blk[:, n:s, :n], blk[:, n:s, n:s]
    t = np.empty((blk.shape[0], s, s), F32)
    t[:, 0::2, 0::2] = ((c00 + c01) + c10) + c11
    t[:, 0::2, 1::2] = ((c00 + c01) - c10) - c11
    t[:, 1::2, 0::2] = ((c00 - c01) + c10) - c11
    t[:, 1::2, 1::2] = ((c00 - c01) - c10) + c11
    blk[:, :s, :s] = t


def transform_to_pixels(acs, coef):
    """TransformToPixels for K blocks of one strategy: coef (K, size) -> (K, 8 * cy, 8 * cx)"""
    k = coef.shape[0]
    cy, cx = ACS_CY[acs], ACS_CX[acs]
    if acs == ACS_DCT or cy * cx > 1:
        return idct2d(coef, 8 * cy, 8 * cx)
    c = coef.reshape(k, 8, 8)
    out = np.empty((k, 8, 8), F32)
    if acs == ACS_DCT2X2:
        blk = c.copy()
        _idct2top(blk, 2)
        _idct2top(blk, 4)
        _idct2top(blk, 8)
        return blk
    if acs == ACS_DCT4X4:
        dcs = _dcs4(c)
        for y in range(2):
            for x in range(2):
                blk = c[:, y::2, x::2].copy()
                blk[:, 0, 0] = dcs[y * 2 + x]
                out[:, 4 * y:4 * y + 4, 4 * x:4 * x + 4] = idct2d(blk.reshape(k, 16), 4, 4)
        return out
    if acs in (ACS_DCT8X4, ACS_DCT4X8):
        b0, b1 = c[:, 0, 0], c[:, 1, 0]
        dcs = [b0 + b1, b0 - b1]
        for i in range(2):
            blk = c[:, i::2, :].copy()
            blk[:, 0, 0] = dcs[i]
            if acs == ACS_DCT8X4:
                out[:, :, 4 * i:4 * i + 4] = idct2d(blk.reshape(k, 32), 8, 4)
            else:
                out[:, 4 * i:4 * i + 4, :] = idct2d(blk.reshape(k, 32), 4, 8)
        return out
    if acs == ACS_IDENTITY:
        dcs = _dcs4(c)
        for y in range(2):
            for x in range(2):
                sub = c[:, y::2, x::2]
                rs = np.zeros(k, F32)
                for iy in range(4):
                    for ix in range(4):
                        if iy or ix:
                            rs = rs + sub[:, iy, ix]
                center = dcs[y * 2 + x] - rs * F32(1.0 / 16)
                o = sub + center[:, None, None]
                o[:, 1, 1] = center
                o[:, 0, 0] = sub[:, 1, 1] + center
                out[:, 4 * y:4 * y + 4, 4 * x:4 * x + 4] = o
        return out
    # AFV0..3
    kind = acs - ACS_AFV0
    afx, afy = kind & 1, kind >> 1
    b00, b01, b10 = c[:, 0, 0], c[:, 0, 1], c[:, 1, 0]
    dcs0 = ((b00 + b10) + b01) * F32(4)
    dcs1 = (b00 + b10) - b01
    dcs2 = b00 - b10
    coeff = c[:, 0::2, 0::2].reshape(k, 16).copy()
    coeff[:, 0] = dcs0
    px = np.zeros((k, 16), F32)
    for j in range(16):
        px = coeff[:, j:j + 1] * AFV_BASIS_F32[j] + px
    blk = px.reshape(k, 4, 4)
    if afy:
        blk = blk[:, ::-1, :]
    if afx:
        blk = blk[:, :, ::-1]
    out[:, 4 * afy:4 * afy + 4, 4 * afx:4 * afx + 4] = blk
    b2 = c[:, 0::2, 1::2].copy()
    b2[:, 0, 0] = dcs1
    xo = 0 if afx else 4
    out[:, 4 * afy:4 * afy + 4, xo:xo + 4] = idct2d(b2.reshape(k, 16), 4, 4)
    b3 = c[:, 1::2, :].copy()
    b3[:, 0, 0] = dcs2
    yo = 0 if afy else 4
    out[:, yo:yo + 4, :] = idct2d(b3.reshape(k, 32), 4, 8)
    return out


# ====================================================================== VarDCT: coefficient orders


def natural_order(acs):
    """CoeffOrderAndLut (order variant): scan position -> coefficient index"""
    cx, cy = ACS_CX[acs], ACS_CY[acs]
    cy, cx = min(cy, cx), max(cy, cx)
    xs = cx // cy
    xsm = xs - 1
    xss = ceil_log2(xs)
    out = [0] * (cx * cy * 64)
    cur = cx * cy
    for i in range(cx * 8):
        for j in range(i + 1):
            x, y = j, i - j
            if i % 2:
                x, y = y, x
            if y & xsm:
                continue
            y >>= xss
            if x < cx and y < cy:
                val = y * cx + x
            else:
                val = cur
                cur += 1
            out[val] = y * cx * 8 + x
    for ip in range(cx * 8 - 1, 0, -1):
        i = ip - 1
        for j in range(i + 1):
            x = cx * 8 - 1 - (i - j)
            y = cx * 8 - 1 - j
            if i % 2:
                x, y = y, x
            if y & xsm:
                continue
            y >>= xss
            out[cur] = y * cx * 8 + x
            cur += 1
    return out


def read_permutation(r, skip, size):
    end = r.read(min(7, perm_ctx(size))) + skip
    if end > size:
        raise JXLError("permutation size")
    lehmer = [0] * size
    last = 0
    for i in range(skip, end):
        lehmer[i] = r.read(min(7, perm_ctx(last)))
        last = lehmer[i]
        if lehmer[i] >= size - i:
            raise JXLError("lehmer code")
    rem = list(range(size))
    return [rem.pop(v) for v in lehmer]


def decode_coeff_orders(b, used_orders, used_acs):
    """DecodeCoeffOrders -> {(order bucket, channel): scan position -> coefficient index}"""
    orders = {}
    r = None
    if used_orders:
        r = Reader(decode_histograms(b, 8), b)
    acs_mask = 0
    for o in range(NUM_ACS):
        if used_acs >> o & 1:
            acs_mask |= 1 << STRATEGY_ORDER[o]
    computed = 0
    for o in range(NUM_ACS):
        ordi = STRATEGY_ORDER[o]
        if computed >> ordi & 1:
            continue
        computed |= 1 << ordi
        used = acs_mask >> ordi & 1
        llf = ACS_CX[o] * ACS_CY[o]
        nat = natural_order(o) if (used or used_orders >> ordi & 1) else None
        if not used_orders >> ordi & 1:
            if used:
                for c in range(3):
                    orders[ordi, c] = nat
        else:
            for c in range(3):
                perm = read_permutation(r, llf, 64 * llf)
                if used:
                    orders[ordi, c] = [nat[p] for p in perm]
    if used_orders and not r.final_ok():
        raise JXLError("coefficient order ANS state")
    return orders


# ====================================================================== VarDCT: quantization tables


def mult_f(v):
    return F32(1) + v if v > 0 else F32(1) / (F32(1) - v)


def quant_weights(rows, cols, bands_in, nb):
    """GetQuantWeights: (3, rows * cols) weights from distance bands"""
    out = np.empty((3, rows * cols), F32)
    for c in range(3):
        bands = [F32(bands_in[c][0])]
        if bands[0] < ALMOST_ZERO:
            raise JXLError("distance bands")
        for i in range(1, nb):
            bands.append(bands[-1] * mult_f(F32(bands_in[c][i])))
            if bands[-1] < ALMOST_ZERO:
                raise JXLError("distance bands")
        scale = F32(nb - 1) / (SQRT2 + F32(1e-6))
        rcpcol = scale / F32(cols - 1)
        rcprow = scale / F32(rows - 1)
        dy = np.arange(rows, dtype=F32)[:, None] * rcprow
        dx = np.arange(cols, dtype=F32)[None, :] * rcpcol
        dist = np.sqrt(dx * dx + dy * dy)
        if nb == 1:
            w = np.full((rows, cols), bands[0], F32)
        else:
            arr = np.array(bands, F32)
            idx = dist.astype(np.int32)
            frac = dist - idx.astype(F32)
            a, bb = arr[idx], arr[idx + 1]
            w = a * fast_powf(bb / a, frac)
        out[c] = w.reshape(-1)
    return out


def interpolate1(pos, mx, bands):
    sp = (pos * F32(len(bands) - 1)) / mx
    idx = int(sp)
    a, bb = bands[idx], bands[idx + 1]
    return a * fast_powf(np.array([bb / a], F32), np.array([sp - F32(idx)], F32))[0]


def quant_table(enc, idx):
    """ComputeQuantTable: (3, size) dequantization multipliers (1 / weight)"""
    rows, cols = 8 * QT_SIZE_X[idx], 8 * QT_SIZE_Y[idx]
    mode = enc[0]
    if mode == 'id':
        w = np.empty((3, 64), F32)
        for c in range(3):
            iw = [F32(v) for v in enc[1][c]]
            w[c, :] = iw[0]
            w[c, 1] = w[c, 8] = iw[1]
            w[c, 9] = iw[2]
    elif mode == 'dct2':
        w = np.empty((3, 64), F32)
        for c in range(3):
            d = [F32(v) for v in enc[1][c]]
            m = np.empty((8, 8), F32)
            m[0, 0] = 0xBAD
            m[0, 1] = m[1, 0] = d[0]
            m[1, 1] = d[1]
            m[0:2, 2:4] = d[2]
            m[2:4, 0:2] = d[2]
            m[2:4, 2:4] = d[3]
            m[0:4, 4:8] = d[4]
            m[4:8, 0:4] = d[4]
            m[4:8, 4:8] = d[5]
            w[c] = m.reshape(64)
    elif mode == 'dct4':
        w4 = quant_weights(4, 4, enc[1][0], enc[1][1])
        w = np.empty((3, 64), F32)
        for c in range(3):
            m = w4[c].reshape(4, 4).repeat(2, 0).repeat(2, 1).reshape(64).copy()
            m0, m1 = F32(enc[2][c][0]), F32(enc[2][c][1])
            m[1] = m[1] / m0
            m[8] = m[8] / m0
            m[9] = m[9] / m1
            w[c] = m
    elif mode == 'dct4x8':
        w48 = quant_weights(4, 8, enc[1][0], enc[1][1])
        w = np.empty((3, 64), F32)
        for c in range(3):
            m = w48[c].reshape(4, 8).repeat(2, 0).reshape(64).copy()
            m[8] = m[8] / F32(enc[2][c])
            w[c] = m
    elif mode == 'dct':
        w = quant_weights(rows, cols, enc[1][0], enc[1][1])
    elif mode == 'raw':
        q = np.array(enc[2], np.int64).astype(F32)
        w = (F32(1) / (F32(enc[1]) * q)).reshape(3, rows * cols)
    elif mode == 'afv':
        afv = [[F32(v) for v in row] for row in enc[1]]
        w48 = quant_weights(4, 8, enc[2][0], enc[2][1])
        w44 = quant_weights(4, 4, enc[3][0], enc[3][1])
        lo = F32(0.8517778890324296)
        hi = (F32(12.97166202570235) - lo) + F32(1e-6)
        w = np.empty((3, 64), F32)
        for c in range(3):
            bands = [afv[c][5]]
            if bands[0] < ALMOST_ZERO:
                raise JXLError("AFV bands")
            for i in range(1, 4):
                bands.append(bands[-1] * mult_f(afv[c][i + 5]))
                if bands[-1] < ALMOST_ZERO:
                    raise JXLError("AFV bands")
            m = np.zeros(64, F32)
            m[0] = 1
            m[8], m[1], m[16], m[2], m[18] = afv[c][0], afv[c][1], afv[c][2], afv[c][3], afv[c][4]
            for y in range(4):
                for x in range(4):
                    if x < 2 and y < 2:
                        continue
                    m[16 * y + 2 * x] = interpolate1(F32(AFV_FREQS[y * 4 + x]) - lo, hi, bands)
            for y in range(4):
                for x in range(8):
                    if x or y:
                        m[(2 * y + 1) * 8 + x] = w48[c][y * 8 + x]
            for y in range(4):
                for x in range(4):
                    if x or y:
                        m[16 * y + 2 * x + 1] = w44[c][y * 4 + x]
            w[c] = m
    else:
        raise JXLError("quant mode")
    if np.any(w >= F32(1.0) / F32(ALMOST_ZERO)) or np.any(w < F32(ALMOST_ZERO)):
        raise JXLError("invalid quantization table")
    return F32(1) / w


def read_dct_params(b):
    nb = b.f(4) + 1
    bands = []
    for _ in range(3):
        v = [F32(b.f16()) for _ in range(nb)]
        if v[0] < ALMOST_ZERO:
            raise JXLError("distance band seed")
        v[0] = v[0] * F32(64)
        bands.append(v)
    return bands, nb


def read_quant_encodings(b, frame):
    """DequantMatrices::Decode: None = library table"""
    enc = [None] * NUM_QUANT_TABLES
    if b.f(1):
        return enc
    for i in range(NUM_QUANT_TABLES):
        mode = b.f(3)
        req = QT_SIZE_X[i] * QT_SIZE_Y[i]
        if 1 <= mode <= 5 and req != 1:
            raise JXLError("quant mode for this table size")
        if mode == 0:
            continue
        if mode in (1, 2):
            n = 3 if mode == 1 else 6
            ws = []
            for c in range(3):
                row = []
                for _ in range(n):
                    v = F32(b.f16())
                    if abs(v) < ALMOST_ZERO:
                        raise JXLError("quantizer too small")
                    row.append(v * F32(64))
                ws.append(row)
            enc[i] = ('id' if mode == 1 else 'dct2', ws)
        elif mode == 3:
            mults = [[F32(b.f16()), F32(b.f16())] for _ in range(3)]
            if any(abs(v) < ALMOST_ZERO for m in mults for v in m):
                raise JXLError("DCT4 multiplier")
            enc[i] = ('dct4', read_dct_params(b), mults)
        elif mode == 4:
            mults = [F32(b.f16()) for _ in range(3)]
            if any(abs(v) < ALMOST_ZERO for v in mults):
                raise JXLError("DCT4X8 multiplier")
            enc[i] = ('dct4x8', read_dct_params(b), mults)
        elif mode == 5:
            afv = []
            for c in range(3):
                row = [F32(b.f16()) for _ in range(9)]
                afv.append([v * F32(64) if k < 6 else v for k, v in enumerate(row)])
            p48 = read_dct_params(b)
            enc[i] = ('afv', afv, p48, read_dct_params(b))
        elif mode == 6:
            enc[i] = ('dct', read_dct_params(b))
        else:
            den = F32(b.f16())
            if den < ALMOST_ZERO:
                raise JXLError("qtable_den")
            wx, hy = 8 * QT_SIZE_X[i], 8 * QT_SIZE_Y[i]
            img = ModImage(wx, hy, 8, [Channel(wx, hy) for _ in range(3)])
            modular_decode(b, img, 1 + 3 * frame.fh.num_dc_groups + i, 0xFFFFFF, frame.gtree, frame.gcode, True)
            q = []
            for ch in img.ch:
                for row in ch.d:
                    for v in row:
                        if v <= 0:
                            raise JXLError("raw quantization table")
                        q.append(v)
            enc[i] = ('raw', den, q)
    return enc


def adjust_quant_bias(q, c, biases):
    """AdjustQuantBias with an exact division (libjxl: q - b3 * rcpps( q ))"""
    qf = q.astype(F32)
    a = np.abs(qf)
    safe = np.where(qf == 0, F32(1), qf)
    big = qf - F32(biases[3]) / safe
    one = np.where(qf > 0, F32(biases[c]), -F32(biases[c]))
    return np.where(a < F32(1.125), np.where(a > 0, one, F32(0)), big).astype(F32)


# ====================================================================== rendering stages


def mirror_pad(p, n):
    return np.pad(p, n, mode='symmetric')


def chroma_up_h(p):
    q = mirror_pad(p, ((0, 0), (1, 1)))
    cur = q[:, 1:-1] * F32(0.75)
    out = np.empty((p.shape[0], p.shape[1] * 2), F32)
    out[:, 0::2] = F32(0.25) * q[:, :-2] + cur
    out[:, 1::2] = F32(0.25) * q[:, 2:] + cur
    return out


def chroma_up_v(p):
    q = mirror_pad(p, ((1, 1), (0, 0)))
    cur = q[1:-1] * F32(0.75)
    out = np.empty((p.shape[0] * 2, p.shape[1]), F32)
    out[0::2] = q[:-2] * F32(0.25) + cur
    out[1::2] = q[2:] * F32(0.25) + cur
    return out


def gaborish(planes, lf):
    out = []
    for c, p in enumerate(planes):
        w1, w2 = F32(lf.gab_w[c][0]), F32(lf.gab_w[c][1])
        w0 = F32(1)
        mul = F32(1) / (w0 + F32(4) * (w1 + w2))
        w0, w1, w2 = w0 * mul, w1 * mul, w2 * mul
        q = mirror_pad(p, 1)
        h, w = p.shape
        sh = lambda dy, dx: q[1 + dy:1 + dy + h, 1 + dx:1 + dx + w]
        s1 = (sh(0, -1) + sh(0, 1)) + (sh(-1, 0) + sh(1, 0))
        s2 = (sh(-1, -1) + sh(-1, 1)) + (sh(1, -1) + sh(1, 1))
        out.append(s2 * w2 + (s1 * w1 + p * w0))
    return out


def epf_sigma_pixels(inv_sigma_blocks, h, w, sm, bsm):
    """per-pixel inv_sigma * sad multiplier and the skip mask"""
    sig = np.repeat(np.repeat(inv_sigma_blocks, 8, 0), 8, 1)[:h, :w]
    ix = np.arange(w) % 8
    iy = np.arange(h) % 8
    mul = np.full((h, w), F32(sm), F32)
    mul[:, (ix == 0) | (ix == 7)] = F32(bsm)
    mul[(iy == 0) | (iy == 7), :] = F32(bsm)
    return sig * mul, sig < MIN_SIGMA


def epf_weight(sad, inv_sigma):
    return np.maximum(sad * inv_sigma + F32(1), F32(0))


def epf_finish(planes, w, acc, skip):
    inv = F32(1) / w
    return [np.where(skip, planes[c], acc[c] * inv) for c in range(3)]


def epf0(planes, lf, inv_sigma_blocks):
    h, w = planes[0].shape
    sm = F32(float(F32(lf.epf_pass0_sigma_scale)) * 1.65)
    inv_sigma, skip = epf_sigma_pixels(inv_sigma_blocks, h, w, sm, sm * F32(lf.epf_border_sad_mul))
    q = [mirror_pad(p, 3) for p in planes]
    sh = lambda c, dy, dx: q[c][3 + dy:3 + dy + h, 3 + dx:3 + dx + w]
    offs = [(-2, 0), (-1, -1), (-1, 0), (-1, 1), (0, -2), (0, -1), (0, 1), (0, 2), (1, -1), (1, 0), (1, 1), (2, 0)]
    plus = [(0, 0), (-1, 0), (0, -1), (1, 0), (0, 1)]
    sads = [np.zeros((h, w), F32) for _ in offs]
    for c in range(3):
        scale = F32(lf.epf_channel_scale[c])
        for i, (oy, ox) in enumerate(offs):
            sad = np.zeros((h, w), F32)
            for py, px in plus:
                sad = sad + np.abs(sh(c, py, px) - sh(c, oy + py, ox + px))
            sads[i] = sad * scale + sads[i]
    wsum = np.ones((h, w), F32)
    acc = [p.copy() for p in planes]
    for i, (oy, ox) in enumerate(offs):
        wt = epf_weight(sads[i], inv_sigma)
        wsum = wsum + wt
        for c in range(3):
            acc[c] = wt * sh(c, oy, ox) + acc[c]
    return epf_finish(planes, wsum, acc, skip)


def epf1(planes, lf, inv_sigma_blocks):
    h, w = planes[0].shape
    sm = F32(1.65)
    inv_sigma, skip = epf_sigma_pixels(inv_sigma_blocks, h, w, sm, sm * F32(lf.epf_border_sad_mul))
    q = [mirror_pad(p, 2) for p in planes]
    sads = [np.zeros((h, w), F32) for _ in range(4)]
    for c in range(3):
        sh = lambda dy, dx: q[c][2 + dy:2 + dy + h, 2 + dx:2 + dx + w]
        p20, p21, p11, p31 = sh(-2, 0), sh(-1, 0), sh(-1, -1), sh(-1, 1)
        p02, p12, p22, p32, p42 = sh(0, -2), sh(0, -1), sh(0, 0), sh(0, 1), sh(0, 2)
        p13, p23, p33, p24 = sh(1, -1), sh(1, 0), sh(1, 1), sh(2, 0)
        s0 = np.abs(p20 - p21)
        s1 = np.abs(p11 - p21)
        s2 = np.abs(p31 - p21)
        s1 = s1 + np.abs(p02 - p12)
        s0 = s0 + np.abs(p11 - p12)
        t = np.abs(p12 - p22)
        s1 = s1 + t
        s2 = s2 + t
        t = np.abs(p22 - p21)
        s3 = t
        s0 = s0 + t
        s0 = s0 + np.abs(p31 - p32)
        t = np.abs(p22 - p32)
        s1 = s1 + t
        s2 = s2 + t
        s2 = s2 + np.abs(p42 - p32)
        s3 = s3 + np.abs(p13 - p12)
        t = np.abs(p22 - p23)
        s0 = s0 + t
        s3 = s3 + t
        s1 = s1 + np.abs(p13 - p23)
        s2 = s2 + np.abs(p33 - p23)
        s3 = s3 + np.abs(p33 - p32)
        s3 = s3 + np.abs(p24 - p23)
        scale = F32(lf.epf_channel_scale[c])
        for i, s in enumerate((s0, s1, s2, s3)):
            sads[i] = s * scale + sads[i]
    wsum = np.ones((h, w), F32)
    acc = [p.copy() for p in planes]
    for i, (oy, ox) in enumerate(((-1, 0), (0, -1), (0, 1), (1, 0))):
        wt = epf_weight(sads[i], inv_sigma)
        wsum = wsum + wt
        for c in range(3):
            acc[c] = wt * q[c][2 + oy:2 + oy + h, 2 + ox:2 + ox + w] + acc[c]
    return epf_finish(planes, wsum, acc, skip)


def epf2(planes, lf, inv_sigma_blocks):
    h, w = planes[0].shape
    sm = F32(float(F32(lf.epf_pass2_sigma_scale)) * 1.65)
    inv_sigma, skip = epf_sigma_pixels(inv_sigma_blocks, h, w, sm, sm * F32(lf.epf_border_sad_mul))
    q = [mirror_pad(p, 1) for p in planes]
    scale = [F32(v) for v in lf.epf_channel_scale]
    wsum = np.ones((h, w), F32)
    acc = [p.copy() for p in planes]
    for oy, ox in ((-1, 0), (0, -1), (0, 1), (1, 0)):
        n = [q[c][1 + oy:1 + oy + h, 1 + ox:1 + ox + w] for c in range(3)]
        sad = np.abs(n[0] - planes[0]) * scale[0]
        sad = np.abs(n[1] - planes[1]) * scale[1] + sad
        sad = np.abs(n[2] - planes[2]) * scale[2] + sad
        wt = epf_weight(sad, inv_sigma)
        wsum = wsum + wt
        for c in range(3):
            acc[c] = wt * n[c] + acc[c]
    return epf_finish(planes, wsum, acc, skip)


def upsample(p, shift, weights):
    """UpsamplingStage: 5x5 kernel per output phase, clamped to the neighbourhood range"""
    n = 1 << shift
    nk = 1 << (shift - 1)
    wt = [F32(v) for v in weights]
    kern = np.zeros((4, 4, 5, 5), F32)
    for i in range(5 * nk):
        for j in range(5 * nk):
            y, x = min(i, j), max(i, j)
            kern[j // 5][i // 5][j % 5][i % 5] = wt[5 * nk * y - y * (y - 1) // 2 + x - y]

    def kernel(ox, oy, ix, iy):
        ix += 2
        iy += 2
        if n == 2:
            return kern[0][0][4 - iy if oy % 2 else iy][4 - ix if ox % 2 else ix]
        hn = n // 2
        ky = oy % hn if oy % n < hn else hn - 1 - oy % hn
        kx = ox % hn if ox % n < hn else hn - 1 - ox % hn
        return kern[ky][kx][iy if oy % n < hn else 4 - iy][ix if ox % n < hn else 4 - ix]

    h, w = p.shape
    q = mirror_pad(p, 2)
    nb = [[q[2 + iy:2 + iy + h, 2 + ix:2 + ix + w] for ix in range(-2, 3)] for iy in range(-2, 3)]
    mn = p.copy()
    mx = p.copy()
    for iy in range(5):
        for ix in range(5):
            mn = np.minimum(nb[iy][ix], mn)
            mx = np.maximum(nb[iy][ix], mx)
    out = np.empty((h * n, w * n), F32)
    for oy in range(n):
        for ox in range(n):
            r = np.zeros((h, w), F32)
            for iy in range(5):
                for ix in range(5):
                    r = kernel(ox, oy, ix - 2, iy - 2) * nb[iy][ix] + r
            out[oy::n, ox::n] = np.minimum(np.maximum(r, mn), mx)
    return out


def srgb_from_linear(x):
    """TF_SRGB::EncodedFromDisplay (rational polynomial of sqrt)"""
    p = [F32(v) for v in (-5.135152395e-04, 5.287254571e-03, 3.903842876e-01, 1.474205315e+00, 7.352629620e-01)]
    q = [F32(v) for v in (1.004519624e-02, 3.036675394e-01, 1.340816930e+00, 9.258482155e-01, 2.424867759e-02)]
    a = np.abs(x)
    s = np.sqrt(a)
    yp, yq = p[4], q[4]
    for k in (3, 2, 1, 0):
        yp = yp * s + p[k]
        yq = yq * s + q[k]
    mag = np.where(a > F32(0.0031308), yp / yq, a * F32(12.92))
    return np.where(x < 0, -mag, mag).astype(F32)


def xyb_to_linear(planes, td, intensity, gray):
    """XybToRgb with the opsin inverse matrix scaled to 255 / intensity_target"""
    inv = [[float(F32(v)) for v in row] for row in td.inv_opsin]
    if gray:
        lum = (0.2126, 0.7152, 0.0722)
        row = [sum(lum[k] * inv[k][i] for k in range(3)) for i in range(3)]
        inv = [row, row, row]
    f = float(F32(255.0) / F32(intensity))
    m = [[F32(inv[j][i] * f) for i in range(3)] for j in range(3)]
    nb = [F32(v) for v in td.opsin_bias]
    cb = [F32(math.copysign(abs(float(v)) ** (1.0 / 3.0), float(v))) for v in nb]
    x, y, b = planes
    g = [(y + x) - cb[0], (y - x) - cb[1], b - cb[2]]
    mixed = [(g[c] * g[c]) * g[c] + nb[c] for c in range(3)]
    out = []
    for j in range(3):
        v = m[j][0] * mixed[0]
        v = m[j][1] * mixed[1] + v
        v = m[j][2] * mixed[2] + v
        out.append(v)
    return out


def ycbcr_to_rgb(planes):
    cb, y, cr = planes
    yv = y + F32(128) / F32(255)
    cgcb = (F32(-0.114) * F32(1.772)) / F32(0.587)
    cgcr = (F32(-0.299) * F32(1.402)) / F32(0.587)
    r = F32(1.402) * cr + yv
    g = cgcr * cr + (cgcb * cb + yv)
    b = F32(1.772) * cb + yv
    return [r, g, b]


TF_709, TF_UNKNOWN, TF_LINEAR, TF_SRGB, TF_PQ, TF_DCI, TF_HLG = 1, 2, 8, 13, 16, 17, 18


def from_linear(planes, ce):
    """FromLinearStage for the output colour encoding"""
    if ce.have_gamma or ce.tf == TF_DCI:
        ig = F32(ce.gamma * 1e-7) if ce.have_gamma else F32(1) / F32(2.6)
        out = []
        for p in planes:
            v = fast_powf(np.maximum(p, F32(1e-5)), np.full(p.shape, ig, F32))
            out.append(np.where(p <= F32(1e-5), F32(0), v).astype(F32))
        return out
    if ce.tf == TF_LINEAR:
        return planes
    if ce.tf == TF_SRGB:
        return [srgb_from_linear(p) for p in planes]
    raise JXLError("transfer function %d not supported yet" % ce.tf)


# ====================================================================== noise synthesis

M64 = (1 << 64) - 1


def splitmix64(z):
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
    return z ^ (z >> 31)


class Xorshift128Plus:
    """8 independent xorshift128+ generators seeded by SplitMix64"""

    def __init__(self, s1, s2, s3, s4):
        a = splitmix64((((s1 << 32) + s2) + 0x9E3779B97F4A7C15) & M64)
        b = splitmix64((((s3 << 32) + s4) + 0x9E3779B97F4A7C15) & M64)
        self.s0, self.s1 = [a], [b]
        for _ in range(7):
            self.s0.append(splitmix64(self.s0[-1]))
            self.s1.append(splitmix64(self.s1[-1]))

    def fill(self):
        """16 random floats in [1, 2)"""
        out = []
        for i in range(8):
            s1, s0 = self.s0[i], self.s1[i]
            bits = (s1 + s0) & M64
            self.s0[i] = s0
            s1 ^= (s1 << 23) & M64
            s1 ^= s0 ^ (s1 >> 18) ^ (s0 >> 5)
            self.s1[i] = s1
            out += [bits & 0xFFFFFFFF, bits >> 32]
        return out


def random_rect(rng, w, h):
    out = np.empty((h, w), np.uint32)
    for y in range(h):
        row = []
        while len(row) < w:
            row += rng.fill()
        out[y] = row[:w]
    return ((out >> 9) | np.uint32(0x3F800000)).view(F32)


def noise_planes(fh, vis, nonvis):
    """the three random planes (PrepareNoiseInput) at the upsampled frame size"""
    w, h = fh.width_up, fh.height_up
    gd, ups = fh.group_dim, fh.upsampling
    planes = [np.zeros((h, w), F32) for _ in range(3)]
    for g in range(fh.num_groups):
        gx, gy = g % fh.xsize_groups, g // fh.xsize_groups
        for iy in range(ups):
            for ix in range(ups):
                x0, y0 = (gx * ups + ix) * gd, (gy * ups + iy) * gd
                if x0 >= w or y0 >= h:
                    continue
                gx1 = min((gx + 1) * gd * ups, w)
                gy1 = min((gy + 1) * gd * ups, h)
                rw, rh = min(gd, gx1 - x0), min(gd, gy1 - y0)
                if rw <= 0 or rh <= 0:
                    continue
                rng = Xorshift128Plus(vis, nonvis, x0, y0)
                for c in range(3):
                    planes[c][y0:y0 + rh, x0:x0 + rw] = random_rect(rng, rw, rh)
    return planes


def convolve_noise(p):
    h, w = p.shape
    q = mirror_pad(p, 2)
    sh = lambda dy, dx: q[2 + dy:2 + dy + h, 2 + dx:2 + dx + w]
    others = np.zeros((h, w), F32)
    for i in range(-2, 3):
        others = others + sh(-2, i)
        others = others + sh(-1, i)
        others = others + sh(1, i)
        others = others + sh(2, i)
    for i in (-2, -1, 1, 2):
        others = others + sh(0, i)
    return others * F32(0.16) + p * F32(-3.84)


def noise_strength(lut, x):
    scaled = np.maximum(F32(0), x * F32(6))
    fl = np.floor(scaled)
    frac = scaled - fl
    top = scaled >= F32(7)
    fl = np.where(top, F32(6), fl)
    frac = np.where(top, F32(1), frac)
    i = fl.astype(np.int32)
    lo, hi = lut[i], lut[i + 1]
    v = (hi - lo) * frac + lo
    return np.maximum(np.minimum(v, F32(1)), F32(0))


def add_noise(planes, rnd, lut, ytox, ytob):
    vx, vy, vb = planes
    lut = np.array(lut, F32)
    ns_g = noise_strength(lut, (vy - vx) * F32(0.5))
    ns_r = noise_strength(lut, (vy + vx) * F32(0.5))
    rr, rg, rc = (r * F32(0.22) for r in rnd)
    red = ns_r * (F32(0.0078125) * rr + F32(0.9921875) * rc)
    green = ns_g * (F32(0.0078125) * rg + F32(0.9921875) * rc)
    rgn = red + green
    return [(ytox * rgn + (red - green)) + vx, vy + rgn, ytob * rgn + vb]


# ====================================================================== blending (patches, frames)

PB_NONE, PB_REPLACE, PB_ADD, PB_MUL, PB_BLEND_ABOVE, PB_BLEND_BELOW, PB_AWADD_ABOVE, PB_AWADD_BELOW = range(8)


def clamp01(v):
    return np.maximum(np.minimum(v, F32(1)), F32(0))


def alpha_blend1(bg, bga, fg, fga, premul, clamp, same):
    """PerformAlphaBlending, single channel (libjxl clamps exactly when clamp is false here)"""
    one = F32(1)
    fa = fga if clamp else clamp01(fga)
    if same:
        return one - (one - fa) * (one - bga)
    if premul:
        return fg + bg * (one - fa)
    na = one - (one - fa) * (one - bga)
    rna = np.where(na > 0, one / np.where(na > 0, na, one), F32(0))
    return (fg * fa + (bg * bga) * (one - fa)) * rna


def alpha_wadd(bg, fg, fga, clamp, same):
    if same:
        return bg.copy()
    return bg + fg * (clamp01(fga) if clamp else fga)


def perform_blending(bg, fg, cb, ecb, extra):
    """PerformBlending: bg / fg lists of 3 + num_ec planes, blendings (mode, alpha channel, clamp)"""
    nec = len(extra)
    has_alpha = any(e.type == ExtraChannelInfo.ALPHA for e in extra)
    out = [None] * (3 + nec)
    for i in range(nec):
        mode, a, cl = ecb[i]
        if mode == PB_ADD:
            out[3 + i] = bg[3 + i] + fg[3 + i]
        elif mode == PB_BLEND_ABOVE:
            out[3 + i] = alpha_blend1(bg[3 + i], bg[3 + a], fg[3 + i], fg[3 + a], extra[a].alpha_associated, cl, i == a)
        elif mode == PB_BLEND_BELOW:
            out[3 + i] = alpha_blend1(fg[3 + i], fg[3 + a], bg[3 + i], bg[3 + a], extra[a].alpha_associated, cl, i == a)
        elif mode == PB_AWADD_ABOVE:
            out[3 + i] = alpha_wadd(bg[3 + i], fg[3 + i], fg[3 + a], cl, i == a)
        elif mode == PB_AWADD_BELOW:
            out[3 + i] = alpha_wadd(fg[3 + i], bg[3 + i], bg[3 + a], cl, i == a)
        elif mode == PB_MUL:
            out[3 + i] = bg[3 + i] * (clamp01(fg[3 + i]) if cl else fg[3 + i])
        elif mode == PB_REPLACE:
            out[3 + i] = fg[3 + i].copy()
        else:
            out[3 + i] = bg[3 + i].copy()
    mode, a, cl = cb
    one = F32(1)
    if mode == PB_ADD or (mode in (PB_AWADD_ABOVE, PB_AWADD_BELOW) and not has_alpha):
        for c in range(3):
            out[c] = bg[c] + fg[c]
    elif mode in (PB_AWADD_ABOVE, PB_AWADD_BELOW):
        lo, hi = (bg, fg) if mode == PB_AWADD_ABOVE else (fg, bg)
        for c in range(3):
            out[c] = alpha_wadd(lo[c], hi[c], hi[3 + a], cl, False)
    elif mode in (PB_BLEND_ABOVE, PB_BLEND_BELOW) and has_alpha:
        lo, hi = (bg, fg) if mode == PB_BLEND_ABOVE else (fg, bg)
        fa = clamp01(hi[3 + a]) if cl else hi[3 + a]
        ba = lo[3 + a]
        if extra[a].alpha_associated:
            for c in range(3):
                out[c] = hi[c] + lo[c] * (one - fa)
            out[3 + a] = one - (one - fa) * (one - ba)
        else:
            na = one - (one - fa) * (one - ba)
            rna = np.where(na > 0, one / np.where(na > 0, na, one), F32(0))
            for c in range(3):
                out[c] = (hi[c] * fa + (lo[c] * ba) * (one - fa)) * rna
            out[3 + a] = na
    elif mode in (PB_BLEND_ABOVE, PB_BLEND_BELOW, PB_REPLACE):
        for c in range(3):
            out[c] = fg[c].copy()
    elif mode == PB_MUL:
        for c in range(3):
            out[c] = bg[c] * (clamp01(fg[c]) if cl else fg[c])
    else:
        for c in range(3):
            out[c] = bg[c].copy()
    return [o.astype(F32) for o in out]


FRAME_TO_PATCH_MODE = {BM_REPLACE: PB_REPLACE, BM_ADD: PB_ADD, BM_MUL: PB_MUL, BM_BLEND: PB_BLEND_ABOVE,
                       BM_AWADD: PB_AWADD_ABOVE}


class RefFrame:
    def __init__(self, planes, xyb):
        self.planes, self.xyb = planes, xyb


# ====================================================================== frames

def pass_bracket(fh, pas):
    """GetDownsamplingBracket -> (minShift, maxShift)"""
    maxs, mins = 2, 3
    i = 0
    while True:
        for j in range(fh.num_downsample):
            if i == fh.last_pass[j]:
                mins = {8: 3, 4: 2, 2: 1, 1: 0}[fh.downsample[j]]
        if i == fh.num_passes - 1:
            mins = 0
        if i == pas:
            return mins, maxs
        maxs = mins - 1
        i += 1


def crop(x0, y0, xs, ys, xend, yend):
    w = 0 if x0 >= xend else min(xs, xend - x0)
    h = 0 if y0 >= yend else min(ys, yend - y0)
    return x0, y0, w, h


class Frame:
    """decoder state of one frame"""

    def __init__(self, cs, b, size, meta, td, is_preview=False, refs=None, counters=None):
        self.cs, self.size, self.meta, self.td = cs, size, meta, td
        self.refs = refs if refs is not None else [None] * 4
        self.ref = None
        self.counters = counters if counters is not None else [0, 0]
        self.fh = fh = FrameHeader().read(b, meta, size, is_preview)
        cnt = self.counters
        if not is_preview and (fh.is_last or fh.duration > 0) and fh.frame_type in (REGULAR, SKIP_PROGRESSIVE):
            cnt[0] += 1
            cnt[1] = 0
        else:
            cnt[1] += 1
        self.vis, self.nonvis = cnt
        n = toc_entries(fh)
        self.sizes, self.offs = read_toc(b, n)
        self.base = b.p >> 3
        self.end = self.base + sum(self.sizes)
        self.single = n == 1
        if self.single:
            self.br = Bits(cs, self.base, self.base + self.sizes[0])

    def section(self, i):
        if self.single:
            return self.br
        st = self.base + self.offs[i]
        return Bits(self.cs, st, st + self.sizes[i])

    def decode(self):
        fh, meta = self.fh, self.meta
        # ---- DC global
        b = self.section(0)
        self.patches = self.read_patches(b) if fh.flags & F_PATCHES else []
        if fh.flags & F_SPLINES:
            raise JXLError("splines not supported yet")
        self.noise_lut = [F32(b.f(10)) / F32(1024) for _ in range(8)] if fh.flags & F_NOISE else None
        if fh.flags & F_USE_DC_FRAME:
            raise JXLError("DC frames not supported yet")
        self.dc_quant = [F32(1.0 / 4096), F32(1.0 / 512), F32(1.0 / 256)]
        if not b.boolean():
            self.dc_quant = [F32(b.f16()) * F32(1.0 / 128) for _ in range(3)]
            if min(self.dc_quant) < ALMOST_ZERO:
                raise JXLError("dc_quant")
        if not fh.modular:
            self.vardct_global(b)
        self.decode_global_modular(b)
        # ---- DC groups
        dcdim = fh.group_dim * 8
        for g in range(fh.num_dc_groups):
            bg = self.section(1 + g)
            if not fh.modular:
                self.vardct_dc_group(g, bg)
            gx, gy = g % fh.xsize_dc_groups, g // fh.xsize_dc_groups
            self.decode_group((gx * dcdim, gy * dcdim, dcdim, dcdim), bg, 3, 1000,
                              1 + fh.num_dc_groups + g)
            if not fh.modular:
                self.ac_metadata(g, bg)
        if not fh.modular and not fh.flags & F_SKIP_ADAPTIVE_DC:
            self.dc_smoothing()
        # ---- AC global
        bg = self.section(1 + fh.num_dc_groups)
        if not fh.modular:
            self.vardct_ac_global(bg)
        # ---- AC groups
        for pas in range(fh.num_passes):
            mins, maxs = pass_bracket(fh, pas)
            for g in range(fh.num_groups):
                bg = self.section(2 + fh.num_dc_groups + pas * fh.num_groups + g)
                if not fh.modular:
                    self.vardct_ac_group(g, pas, bg)
                gx, gy = g % fh.xsize_groups, g // fh.xsize_groups
                gd = fh.group_dim
                self.decode_group((gx * gd, gy * gd, gd, gd), bg, mins, maxs,
                                  1 + 3 * fh.num_dc_groups + NUM_QUANT_TABLES + fh.num_groups * pas + g)
        if not fh.modular:
            self.vardct_pixels()
        self.full.undo_transforms(self.ghdr.wp if self.ghdr else WPHeader().read(Bits(b"\x01")))
        return self.render()

    # ---------------------------------------------------------------- VarDCT

    def read_patches(self, b):
        """PatchDictionary::Decode: (ref, x0, y0, xsize, ysize, x, y, blendings) per patch"""
        fh = self.fh
        nec = len(self.meta.extra)
        r = Reader(decode_histograms(b, 10), b)
        xs, ys = fh.xsize_blocks * 8, fh.ysize_blocks * 8
        num_ref = r.read(0)
        if num_ref > 1024 + xs * ys // 4:
            raise JXLError("too many reference patches")
        out = []
        for _ in range(num_ref):
            ref = r.read(1)
            if ref >= 4 or self.refs[ref] is None:
                raise JXLError("patch reference frame")
            rf = self.refs[ref]
            if not rf.xyb:
                raise JXLError("patches from a frame saved after the colour transform")
            rx0, ry0 = r.read(3), r.read(3)
            rw, rh = r.read(2) + 1, r.read(2) + 1
            if rx0 + rw > rf.planes[0].shape[1] or ry0 + rh > rf.planes[0].shape[0]:
                raise JXLError("patch outside its reference frame")
            cnt = r.read(7) + 1
            x = y = 0
            for i in range(cnt):
                if i == 0:
                    x, y = r.read(4), r.read(4)
                else:
                    x += unpack_signed(r.read(6))
                    y += unpack_signed(r.read(6))
                    if x < 0 or y < 0:
                        raise JXLError("negative patch position")
                if x + rw > xs or y + rh > ys:
                    raise JXLError("patch outside the frame")
                bl = []
                for _ in range(nec + 1):
                    mode = r.read(5)
                    if mode >= 8:
                        raise JXLError("patch blend mode")
                    alpha = 0
                    if mode >= PB_BLEND_ABOVE and nec > 1:
                        alpha = r.read(8)
                        if alpha >= nec:
                            raise JXLError("patch alpha channel")
                    clamp = bool(r.read(9)) if mode >= PB_MUL else False
                    bl.append((mode, alpha, clamp))
                out.append((ref, rx0, ry0, rw, rh, x, y, bl))
        if not r.final_ok():
            raise JXLError("patch dictionary ANS state")
        return out

    def apply_patches(self, chans):
        h, w = chans[0].shape
        for ref, rx0, ry0, rw, rh, x, y, bl in self.patches:
            if x >= w or y >= h:
                continue
            x1, y1 = min(x + rw, w), min(y + rh, h)
            fg = [pl[ry0:ry0 + y1 - y, rx0:rx0 + x1 - x] for pl in self.refs[ref].planes]
            bg = [c[y:y1, x:x1] for c in chans]
            for c, o in zip(chans, perform_blending(bg, fg, bl[0], bl[1:], self.meta.extra)):
                c[y:y1, x:x1] = o

    def ytox_ratio(self, f):
        return self.base_x + F32(f) * self.color_scale

    def ytob_ratio(self, f):
        return self.base_b + F32(f) * self.color_scale

    def vardct_global(self, b):
        """quantizer, block context map, colour correlation DC (DecodeGlobalDCInfo)"""
        fh = self.fh
        self.global_scale = b.u32(B(11, 1), B(11, 2049), B(12, 4097), B(16, 8193))
        self.quant_dc = b.u32(V(16), B(5, 1), B(8, 1), B(16, 1))
        self.inv_global_scale = F32(65536.0 / self.global_scale)
        self.global_scale_f = F32(self.global_scale * (1.0 / 65536))
        iqdc = self.inv_global_scale / F32(self.quant_dc)
        self.mul_dc = [iqdc * self.dc_quant[c] for c in range(3)]
        if b.f(1):
            self.dc_thr = [[], [], []]
            self.qf_thr = []
            self.num_dc_ctxs = 1
            self.ctx_map = list(DEFAULT_CTX_MAP)
        else:
            self.dc_thr = []
            self.num_dc_ctxs = 1
            for _ in range(3):
                n = b.f(4)
                self.dc_thr.append([unpack_signed(b.u32(B(4), B(8, 16), B(16, 272), B(32, 65808)))
                                    for _ in range(n)])
                self.num_dc_ctxs *= n + 1
            n = b.f(4)
            self.qf_thr = [b.u32(B(2), B(3, 4), B(5, 12), B(8, 44)) + 1 for _ in range(n)]
            if self.num_dc_ctxs * (len(self.qf_thr) + 1) > 64:
                raise JXLError("block context map too big")
            self.ctx_map = decode_context_map(b, 3 * NUM_ORDERS * self.num_dc_ctxs * (len(self.qf_thr) + 1))
        self.num_ctxs = max(self.ctx_map) + 1
        if self.num_ctxs > 16:
            raise JXLError("too many block contexts")
        self.color_factor = 84
        self.base_x = F32(0)
        self.base_b = F32(1)          # the decoder's ColorCorrelationMap is always created with XYB = true
        self.ytox_dc = self.ytob_dc = 0
        if not b.f(1):
            self.color_factor = b.u32(V(84), V(256), B(8, 2), B(16, 258))
            self.base_x = F32(b.f16())
            self.base_b = F32(b.f16())
            if abs(self.base_x) > 4 or abs(self.base_b) > 4:
                raise JXLError("base correlation")
            self.ytox_dc = b.f(8) - 128
            self.ytob_dc = b.f(8) - 128
        self.color_scale = F32(1) / F32(self.color_factor)
        self.cfl_dc = [self.ytox_ratio(self.ytox_dc), F32(0), self.ytob_ratio(self.ytob_dc)]
        xb, yb = fh.xsize_blocks, fh.ysize_blocks
        self.dc = [np.zeros((yb, xb), F32) for _ in range(3)]
        self.qdc = np.zeros((yb, xb), np.int32)
        self.acs = [[None] * xb for _ in range(yb)]
        self.qf = np.zeros((yb, xb), np.int32)
        self.sharp = np.zeros((yb, xb), np.int32)
        self.inv_sigma = np.zeros((yb, xb), F32)
        self.ytox = np.zeros(((yb + 7) >> 3, (xb + 7) >> 3), np.int32)
        self.ytob = np.zeros(((yb + 7) >> 3, (xb + 7) >> 3), np.int32)
        self.used_acs = 0
        self.is444 = all(fh.hshift(c) == 0 and fh.vshift(c) == 0 for c in range(3))

    def dc_group_rect(self, g):
        fh = self.fh
        gx, gy = g % fh.xsize_dc_groups, g // fh.xsize_dc_groups
        x0, y0 = gx * fh.group_dim, gy * fh.group_dim
        return x0, y0, min(fh.group_dim, fh.xsize_blocks - x0), min(fh.group_dim, fh.ysize_blocks - y0)

    def vardct_dc_group(self, g, b):
        """DecodeVarDCTDC + DequantDC"""
        fh = self.fh
        x0, y0, w, h = self.dc_group_rect(g)
        extra = b.f(2)
        mul = F32(1.0 / (1 << extra))
        chans = [Channel(w >> fh.hshift(c), h >> fh.vshift(c)) for c in (1, 0, 2)]
        img = ModImage(w, h, self.meta.bit_depth.bits, chans)
        modular_decode(b, img, 1 + g, 0xFFFFFF, self.gtree, self.gcode, True)
        q = {1: img.ch[0].d, 0: img.ch[1].d, 2: img.ch[2].d}
        qa = {c: np.array(q[c], np.int64).reshape(len(q[c]), -1).astype(F32) for c in range(3)}
        if self.is444:
            fx, fy, fb = (self.mul_dc[c] * mul for c in range(3))
            in_x, in_y, in_b = qa[0] * fx, qa[1] * fy, qa[2] * fb
            self.dc[1][y0:y0 + h, x0:x0 + w] = in_y
            self.dc[0][y0:y0 + h, x0:x0 + w] = in_y * self.cfl_dc[0] + in_x
            self.dc[2][y0:y0 + h, x0:x0 + w] = in_y * self.cfl_dc[2] + in_b
        else:
            for c in (1, 0, 2):
                hs, vs = fh.hshift(c), fh.vshift(c)
                self.dc[c][y0 >> vs:(y0 >> vs) + (h >> vs), x0 >> hs:(x0 >> hs) + (w >> hs)] = \
                    qa[c] * (self.mul_dc[c] * mul)
        if self.num_dc_ctxs > 1:
            t = self.dc_thr
            for y in range(h):
                for x in range(w):
                    bk = [sum(1 for th in t[c] if q[c][y >> fh.vshift(c)][x >> fh.hshift(c)] > th)
                          for c in range(3)]
                    self.qdc[y0 + y, x0 + x] = ((bk[0] * (len(t[2]) + 1) + bk[2]) * (len(t[1]) + 1)) + bk[1]

    def ac_metadata(self, g, b):
        """DecodeAcMetadata: CfL maps, AC strategy, quant field, EPF sharpness"""
        fh = self.fh
        x0, y0, w, h = self.dc_group_rect(g)
        count = b.f(ceil_log2(w * h)) + 1
        cw, chh = (w + 7) >> 3, (h + 7) >> 3
        chans = [Channel(cw, chh, 3, 3), Channel(cw, chh, 3, 3), Channel(count, 2), Channel(w, h)]
        img = ModImage(w, h, self.meta.bit_depth.bits, chans)
        modular_decode(b, img, 1 + 2 * fh.num_dc_groups + g, 0xFFFFFF, self.gtree, self.gcode, True)
        cx0, cy0 = x0 >> 3, y0 >> 3
        for y in range(chh):
            for x in range(cw):
                self.ytox[cy0 + y, cx0 + x] = max(-128, min(127, img.ch[0].d[y][x]))
                self.ytob[cy0 + y, cx0 + x] = max(-128, min(127, img.ch[1].d[y][x]))
        acs_row, qf_row = img.ch[2].d
        sharp = img.ch[3].d
        xlim, ylim = min(fh.xsize_blocks, x0 + w), min(fh.ysize_blocks, y0 + h)
        num = 0
        for iy in range(h):
            y = y0 + iy
            for ix in range(w):
                x = x0 + ix
                s = sharp[iy][ix]
                if s < 0 or s >= 8:
                    raise JXLError("sharpness")
                self.sharp[y, x] = s
                if self.acs[y][x] is not None:
                    continue
                if num >= count:
                    raise JXLError("AC strategy count")
                a = acs_row[num]
                if not 0 <= a < NUM_ACS:
                    raise JXLError("AC strategy")
                self.used_acs |= 1 << a
                bx, by = ACS_CX[a], ACS_CY[a]
                if (bx > 1 or by > 1) and not self.is444:
                    raise JXLError("AC strategy with chroma subsampling")
                if x + bx > (x // GROUP_BLOCKS + 1) * GROUP_BLOCKS or x + bx > xlim:
                    raise JXLError("AC strategy x overflow")
                if y + by > (y // GROUP_BLOCKS + 1) * GROUP_BLOCKS or y + by > ylim:
                    raise JXLError("AC strategy y overflow")
                for yy in range(by):
                    for xx in range(bx):
                        if self.acs[y + yy][x + xx] is not None:
                            raise JXLError("AC strategy overlap")
                        self.acs[y + yy][x + xx] = (a, yy == 0 and xx == 0)
                self.qf[y, x] = 1 + max(0, min(255, qf_row[num]))
                num += 1
        if fh.lf.epf_iters > 0:
            lf = fh.lf
            for iy in range(h):
                for ix in range(w):
                    a, first = self.acs[y0 + iy][x0 + ix]
                    if not first:
                        continue
                    sq = F32(lf.epf_quant_mul) / ((self.global_scale_f * F32(self.qf[y0 + iy, x0 + ix]))
                                                  * INV_SIGMA_NUM)
                    for yy in range(ACS_CY[a]):
                        for xx in range(ACS_CX[a]):
                            s = sq * F32(lf.epf_sharp_lut[self.sharp[y0 + iy + yy, x0 + ix + xx]])
                            s = min(F32(-1e-4), s)
                            self.inv_sigma[y0 + iy + yy, x0 + ix + xx] = F32(1) / s

    def dc_smoothing(self):
        """AdaptiveDCSmoothing"""
        h, w = self.dc[0].shape
        if h <= 2 or w <= 2:
            return
        w1, w2 = F32(0.20345139757231578), F32(0.0334829185968739)
        w0 = F32(1) - F32(4) * (w1 + w2)
        gap = np.full((h - 2, w - 2), F32(0.5))
        mcs, sms = [], []
        for c in range(3):
            p = self.dc[c]
            mc = p[1:-1, 1:-1]
            corner = (p[:-2, :-2] + p[:-2, 2:]) + (p[2:, :-2] + p[2:, 2:])
            side = (p[1:-1, :-2] + p[1:-1, 2:]) + (p[:-2, 1:-1] + p[2:, 1:-1])
            sm = corner * w2 + (side * w1 + mc * w0)
            gap = np.maximum(gap, np.abs((mc - sm) / self.mul_dc[c]))
            mcs.append(mc)
            sms.append(sm)
        factor = np.maximum(F32(-4) * gap + F32(3), F32(0))
        for c in range(3):
            out = self.dc[c].copy()
            out[1:-1, 1:-1] = (sms[c] - mcs[c]) * factor + mcs[c]
            self.dc[c] = out

    def vardct_ac_global(self, b):
        fh = self.fh
        self.qenc = read_quant_encodings(b, self)
        self.qtables = {}
        self.num_histograms = 1 + b.f(ceil_log2(fh.num_groups))
        self.num_ac_ctx = self.num_ctxs * (NONZERO_BUCKETS + ZERO_DENSITY_CONTEXTS)
        self.orders, self.ac_code = [], []
        for _ in range(fh.num_passes):
            used_orders = b.u32(V(0x5F), V(0x13), V(0), B(NUM_ORDERS))
            self.orders.append(decode_coeff_orders(b, used_orders, self.used_acs))
            self.ac_code.append(decode_histograms(b, self.num_histograms * self.num_ac_ctx))
        self.coeffs = {}
        self.x_dm = F32(float(F32(1) / F32(1.25)) ** (fh.x_qm_scale - 2))
        self.b_dm = F32(float(F32(1) / F32(1.25)) ** (fh.b_qm_scale - 2))

    def group_block_rect(self, g):
        fh = self.fh
        gx, gy = g % fh.xsize_groups, g // fh.xsize_groups
        x0, y0 = gx * GROUP_BLOCKS, gy * GROUP_BLOCKS
        return x0, y0, min(GROUP_BLOCKS, fh.xsize_blocks - x0), min(GROUP_BLOCKS, fh.ysize_blocks - y0)

    def vardct_ac_group(self, g, pas, b):
        """the coefficients of one pass of one group (GetBlockFromBitstream / DecodeACVarBlock)"""
        fh = self.fh
        x0, y0, w, h = self.group_block_rect(g)
        nbits = ceil_log2(self.num_histograms)
        sel = b.f(nbits) if nbits else 0
        if sel >= self.num_histograms:
            raise JXLError("histogram selector")
        ctx_off = sel * self.num_ac_ctx
        r = Reader(self.ac_code[pas], b)
        orders = self.orders[pas]
        shift = fh.shift[pas]
        nz = [[[0] * GROUP_BLOCKS for _ in range(GROUP_BLOCKS)] for _ in range(3)]
        hs = [fh.hshift(c) for c in range(3)]
        vs = [fh.vshift(c) for c in range(3)]
        store = self.coeffs.setdefault(g, {})
        nctx = self.num_ctxs
        for by in range(h):
            for bx in range(w):
                a, first = self.acs[y0 + by][x0 + bx]
                if not first:
                    continue
                log2 = ACS_LOG2[a]
                cov = 1 << log2
                size = cov * 64
                blocks = store.get((bx, by))
                if blocks is None:
                    blocks = store[bx, by] = [[0] * size for _ in range(3)]
                ordi = STRATEGY_ORDER[a]
                for c in (1, 0, 2):
                    sbx, sby = bx >> hs[c], by >> vs[c]
                    if sbx << hs[c] != bx or sby << vs[c] != by:
                        continue
                    row = nz[c][sby]
                    top = nz[c][sby - 1] if sby else None
                    if sbx == 0:
                        pred = 32 if top is None else top[0]
                    elif top is None:
                        pred = row[sbx - 1]
                    else:
                        pred = (top[sbx] + row[sbx - 1] + 1) >> 1
                    qfv = int(self.qf[y0 + by, x0 + sbx])
                    qf_idx = sum(1 for t in self.qf_thr if qfv > t)
                    idx = (c ^ 1 if c < 2 else 2) * NUM_ORDERS + ordi
                    idx = idx * (len(self.qf_thr) + 1) + qf_idx
                    idx = idx * self.num_dc_ctxs + int(self.qdc[y0 + by, x0 + bx])
                    bctx = self.ctx_map[idx]
                    nzv = min(pred, 64)
                    nzc = (nzv if nzv < 8 else 4 + nzv // 2) * nctx + bctx
                    nzeros = r.read(nzc + ctx_off)
                    if nzeros > size - cov:
                        raise JXLError("nzeros")
                    v = (nzeros + cov - 1) >> log2
                    for yy in range(ACS_CY[a]):
                        for xx in range(ACS_CX[a]):
                            nz[c][sby + yy][sbx + xx] = v
                    histo = ctx_off + nctx * NONZERO_BUCKETS + ZERO_DENSITY_CONTEXTS * bctx
                    order = orders[ordi, c]
                    blk = blocks[c]
                    prev = 0 if nzeros > size // 16 else 1
                    k = cov
                    while k < size and nzeros:
                        ctx = histo + (COEFF_NZ_CTX[(nzeros + cov - 1) >> log2] + COEFF_FREQ_CTX[k >> log2]) * 2 + prev
                        u = r.read(ctx)
                        blk[order[k]] += unpack_signed(u) << shift
                        prev = 1 if u else 0
                        nzeros -= prev
                        k += 1
                    if nzeros:
                        raise JXLError("nzeros left at the end of a block")
        if not r.final_ok():
            raise JXLError("AC ANS state")

    def qtable(self, a):
        t = ACS_QTABLE[a]
        m = self.qtables.get(t)
        if m is None:
            enc = self.qenc[t] or QUANT_LIBRARY[t]
            m = self.qtables[t] = quant_table(enc, t)
        return m

    def vardct_pixels(self):
        """dequantization, LLF from DC and inverse transforms of every block (DecodeGroupImpl)"""
        fh = self.fh
        hs = [fh.hshift(c) for c in range(3)]
        vs = [fh.vshift(c) for c in range(3)]
        self.px = [np.zeros(((fh.ysize_blocks >> vs[c]) * 8, (fh.xsize_blocks >> hs[c]) * 8), F32)
                   for c in range(3)]
        qb = self.td.quant_bias
        batches = {}
        for g in range(fh.num_groups):
            x0, y0, w, h = self.group_block_rect(g)
            store = self.coeffs.get(g, {})
            for by in range(h):
                for bx in range(w):
                    a, first = self.acs[y0 + by][x0 + bx]
                    if not first:
                        continue
                    cy, cx = ACS_CY[a], ACS_CX[a]
                    size = 64 * cx * cy
                    blocks = store.get((bx, by)) or [[0] * size for _ in range(3)]
                    ty, tx = (y0 + by) >> 3, (x0 + bx) >> 3
                    x_cc = self.ytox_ratio(int(self.ytox[ty, tx]))
                    b_cc = self.ytob_ratio(int(self.ytob[ty, tx]))
                    sds = self.inv_global_scale / F32(int(self.qf[y0 + by, x0 + bx]))
                    mats = self.qtable(a)
                    xm = mats[0] * (sds * self.x_dm)
                    ym = mats[1] * sds
                    bm = mats[2] * (sds * self.b_dm)
                    q = [np.array(blk, np.int64) for blk in blocks]
                    dy = adjust_quant_bias(q[1], 1, qb) * ym
                    dx = x_cc * dy + adjust_quant_bias(q[0], 0, qb) * xm
                    db = b_cc * dy + adjust_quant_bias(q[2], 2, qb) * bm
                    co = [dx, dy, db]
                    mn, mx = min(cy, cx), max(cy, cx)
                    for c in range(3):
                        sbx, sby = (x0 >> hs[c]) + (bx >> hs[c]), (y0 >> vs[c]) + (by >> vs[c])
                        if mn * mx == 1:
                            co[c][0] = self.dc[c][sby, sbx]
                        else:
                            llf = llf_from_dc(a, self.dc[c][sby:sby + cy, sbx:sbx + cx])
                            co[c].reshape(8 * mn, 8 * mx)[:mn, :mx] = llf
                    for c in (1, 0, 2):
                        if (bx >> hs[c]) << hs[c] != bx or (by >> vs[c]) << vs[c] != by:
                            continue
                        sbx, sby = (x0 >> hs[c]) + (bx >> hs[c]), (y0 >> vs[c]) + (by >> vs[c])
                        batches.setdefault((a, c), []).append((co[c], sbx * 8, sby * 8))
        for (a, c), items in batches.items():
            pix = transform_to_pixels(a, np.stack([it[0] for it in items]))
            ph, pw = 8 * ACS_CY[a], 8 * ACS_CX[a]
            for (co, px, py), p in zip(items, pix):
                self.px[c][py:py + ph, px:px + pw] = p

    # ---------------------------------------------------------------- modular global / groups

    def decode_global_modular(self, b):
        fh, meta = self.fh, self.meta
        nb = 3
        if meta.color.color_space == ColorEncoding.GRAY and fh.color_transform == 'none':
            nb = 1
        if not fh.modular:
            nb = 0
        self.nb_color = nb
        nx = len(meta.extra)
        self.gtree = self.gcode = None
        if b.f(1):
            limit = min(1 << 22, 1024 + fh.width * fh.height * (nb + nx) // 16)
            self.gtree = decode_tree(b, limit)
            self.gcode = decode_histograms(b, (len(self.gtree.nodes) + 1) // 2)
        chans = []
        for c in range(nb):
            ch = Channel(fh.width, fh.height)
            if fh.color_transform == 'ycbcr':
                ch.hshift, ch.vshift = fh.hshift(c), fh.vshift(c)
                ch = Channel(-(-fh.width // (1 << ch.hshift)), -(-fh.height // (1 << ch.vshift)), ch.hshift,
                             ch.vshift)
            chans.append(ch)
        for i, e in enumerate(meta.extra):
            ups = fh.ec_upsampling[i]
            sh = ceil_log2(ups) - ceil_log2(fh.upsampling)
            chans.append(Channel(-(-fh.width_up // ups), -(-fh.height_up // ups), sh, sh))
        self.full = ModImage(fh.width, fh.height, meta.bit_depth.bits, chans)
        self.ghdr = modular_decode(b, self.full, 0, fh.group_dim, self.gtree, self.gcode, False)

    def decode_group(self, rect, b, mins, maxs, stream_id):
        fh = self.fh
        full = self.full
        x0, y0, xs, ys = rect
        c = full.nb_meta
        while c < len(full.ch):
            fc = full.ch[c]
            if fc.w > fh.group_dim or fc.h > fh.group_dim:
                break
            c += 1
        chans, where = [], []
        for c in range(c, len(full.ch)):
            fc = full.ch[c]
            sh = min(fc.hshift, fc.vshift)
            if sh > maxs or sh < mins:
                continue
            r = crop(x0 >> fc.hshift, y0 >> fc.vshift, xs >> fc.hshift, ys >> fc.vshift, fc.w, fc.h)
            if r[2] == 0 or r[3] == 0:
                continue
            chans.append(Channel(r[2], r[3], fc.hshift, fc.vshift))
            where.append((c, r))
        if not chans:
            return
        gi = ModImage(xs, ys, full.bitdepth, chans)
        modular_decode(b, gi, stream_id, 0xFFFFFF, self.gtree, self.gcode, True)
        for gc, (c, r) in zip(gi.ch, where):
            fc = full.ch[c]
            for yy in range(r[3]):
                fc.d[r[1] + yy][r[0]:r[0] + r[2]] = gc.d[yy][:r[2]]

    def modular_color(self):
        """ModularImageToDecodedRect for the colour channels: float32 planes (chroma possibly subsampled)"""
        fh, meta = self.fh, self.meta
        full = self.full
        if meta.bit_depth.float and fh.color_transform != 'xyb':
            raise JXLError("floating point modular samples not supported yet")
        ints = lambda c: np.array(full.ch[c].d, np.int64).reshape(full.ch[c].h, full.ch[c].w)
        if fh.color_transform == 'xyb':
            y = ints(0)
            return [ints(1).astype(F32) * self.dc_quant[0], y.astype(F32) * self.dc_quant[1],
                    (ints(2) + y).astype(F32) * self.dc_quant[2]]
        f = F32(1.0 / ((1 << full.bitdepth) - 1))
        if self.nb_color == 1:
            g = ints(0).astype(F32) * f
            return [g, g.copy(), g.copy()]
        return [ints(c).astype(F32) * f for c in range(3)]

    def ec_planes(self):
        out = []
        for i, e in enumerate(self.meta.extra):
            ch = self.full.ch[self.nb_color + i]
            if e.bit_depth.float:
                raise JXLError("floating point extra channels not supported yet")
            out.append(np.array(ch.d, np.int64).reshape(ch.h, ch.w).astype(F32)
                       * F32(1.0 / ((1 << e.bit_depth.bits) - 1)))
        return out

    def can_be_referenced(self):
        fh = self.fh
        return not fh.is_last and fh.frame_type != DC_FRAME and (fh.duration == 0 or fh.save_as_reference != 0)

    def needs_blending(self):
        fh = self.fh
        if fh.frame_type not in (REGULAR, SKIP_PROGRESSIVE):
            return False
        return fh.custom_size or fh.blending.mode != BM_REPLACE or any(e.mode != BM_REPLACE for e in fh.ec_blending)

    def up_weights(self, shift):
        return self.td.up_weights.get(1 << (shift - 1)) or (UP2_W, UP4_W, UP8_W)[shift - 1]

    def render(self):
        """the render pipeline: colour planes (3 float32 arrays, 0..1 nominal) and the alpha plane or None"""
        fh, meta = self.fh, self.meta
        lf = fh.lf
        if fh.modular:
            planes = self.modular_color()
        else:
            planes = [self.px[c][:-(-fh.height >> fh.vshift(c)), :-(-fh.width >> fh.hshift(c))]
                      for c in range(3)]
        for c in range(3):
            if fh.hshift(c):
                planes[c] = chroma_up_h(planes[c])
            if fh.vshift(c):
                planes[c] = chroma_up_v(planes[c])
        planes = [p[:fh.height, :fh.width] for p in planes]
        if lf.gab:
            planes = gaborish(planes, lf)
        if lf.epf_iters:
            if fh.modular:
                sig = np.full((fh.ysize_blocks, fh.xsize_blocks), INV_SIGMA_NUM / F32(lf.epf_sigma_for_modular), F32)
            else:
                sig = self.inv_sigma
            if lf.epf_iters >= 3:
                planes = epf0(planes, lf, sig)
            planes = epf1(planes, lf, sig)
            if lf.epf_iters >= 2:
                planes = epf2(planes, lf, sig)
        ecs = self.ec_planes()
        late = fh.upsampling != 1 and all(u == fh.upsampling for u in fh.ec_upsampling)
        if not late:
            ecs = [upsample(e, ceil_log2(u), self.up_weights(ceil_log2(u))) if u != 1 else e
                   for e, u in zip(ecs, fh.ec_upsampling)]
        if self.patches:
            chans = [p.copy() for p in planes + ecs]
            self.apply_patches(chans)
            planes, ecs = chans[:3], chans[3:]
        if fh.upsampling != 1:
            sh = ceil_log2(fh.upsampling)
            planes = [upsample(p, sh, self.up_weights(sh)) for p in planes]
            if late:
                ecs = [upsample(e, sh, self.up_weights(sh)) for e in ecs]
        planes = [p[:fh.height_up, :fh.width_up] for p in planes]
        if self.noise_lut is not None:
            rnd = [convolve_noise(r) for r in noise_planes(fh, self.vis, self.nonvis)]
            if fh.modular:
                ytox, ytob = F32(0), F32(1)
            else:
                ytox, ytob = self.base_x, self.base_b
            planes = add_noise(planes, rnd, self.noise_lut, ytox, ytob)
        ecs = [e[:fh.height_up, :fh.width_up] for e in ecs]
        can_ref = self.can_be_referenced()
        if can_ref and fh.save_before_ct:
            self.ref = RefFrame([p.copy() for p in planes + ecs], True)
        ce = meta.color
        linear = False
        to_output = False
        if fh.color_transform == 'ycbcr':
            planes = ycbcr_to_rgb(planes)
        elif fh.color_transform == 'xyb':
            gray = ce.color_space == ColorEncoding.GRAY
            to_output = not ce.want_icc and (not gray or ce.white_point == 1) and \
                (ce.have_gamma or ce.tf in (TF_709, TF_LINEAR, TF_SRGB, TF_PQ, TF_DCI, TF_HLG))
            if to_output and not gray and (ce.primaries != 1 or ce.white_point != 1):
                raise JXLError("XYB to non-sRGB primaries not supported yet")
            planes = xyb_to_linear(planes, self.td, meta.tone.intensity_target, gray)
            linear = True
        if self.needs_blending():
            if linear and to_output:
                planes = from_linear(planes, ce)
            linear = False
            planes, ecs = self.blend(planes, ecs)
        if can_ref and not fh.save_before_ct:
            if linear and to_output:
                planes = from_linear(planes, ce)
            linear = False
            self.ref = RefFrame([p.copy() for p in planes + ecs], False)
        if linear and to_output:
            planes = from_linear(planes, ce)
        alpha = None
        for i, e in enumerate(meta.extra):
            if e.type == ExtraChannelInfo.ALPHA:
                alpha = ecs[i]
                break
        self.planes, self.ecs, self.alpha = planes, ecs, alpha
        return planes, alpha

    def blend(self, planes, ecs):
        """BlendingStage: the frame on the canvas (image size) over reference frame( s )"""
        fh, meta = self.fh, self.meta
        W, H = self.size.xsize, self.size.ysize
        srcs = [fh.blending.source] * 3 + [e.source for e in fh.ec_blending]
        canvas = []
        for c, src in enumerate(srcs):
            rf = self.refs[src]
            if rf is None:
                canvas.append(np.zeros((H, W), F32))
                continue
            if rf.xyb:
                raise JXLError("blending onto an XYB reference frame")
            pl = rf.planes[c]
            if pl.shape[0] < H or pl.shape[1] < W:
                raise JXLError("blending onto a cropped reference frame")
            canvas.append(pl[:H, :W].copy())
        fx0, fy0 = (fh.x0, fh.y0) if fh.custom_size else (0, 0)
        fw, fhh = planes[0].shape[1], planes[0].shape[0]
        cx0, cy0, cx1, cy1 = max(fx0, 0), max(fy0, 0), min(fx0 + fw, W), min(fy0 + fhh, H)
        if cx1 > cx0 and cy1 > cy0:
            fg = [p[cy0 - fy0:cy1 - fy0, cx0 - fx0:cx1 - fx0] for p in planes + ecs]
            bg = [c[cy0:cy1, cx0:cx1] for c in canvas]
            mk = lambda bi: (FRAME_TO_PATCH_MODE[bi.mode], bi.alpha_channel, bi.clamp)
            out = perform_blending(bg, fg, mk(fh.blending), [mk(e) for e in fh.ec_blending], meta.extra)
            for c, o in zip(canvas, out):
                c[cy0:cy1, cx0:cx1] = o
        return canvas[:3], canvas[3:]


def decode_frames(data, first_only=False):
    """all displayed frames (coalesced on the canvas): (size, meta, [(rgb planes, alpha, duration)])"""
    cs = codestream(data)
    b, size, meta, td, icc = read_headers(cs)
    counters = [0, 0]
    if meta.have_preview:
        pf = Frame(cs, b, size, meta, td, True, counters=counters)
        b = Bits(cs, pf.end)
    refs = [None] * 4
    shown = []
    while True:
        fr = Frame(cs, b, size, meta, td, refs=refs, counters=counters)
        rgb, alpha = fr.decode()
        global LAST_FRAME
        LAST_FRAME = fr
        if fr.ref is not None:
            refs[fr.fh.save_as_reference] = fr.ref
        if fr.fh.frame_type in (REGULAR, SKIP_PROGRESSIVE) and (fr.fh.is_last or fr.fh.duration > 0):
            shown.append((rgb, alpha, fr.fh.duration))
            if first_only:
                break
        if fr.fh.is_last or fr.end >= len(cs):
            break
        b = Bits(cs, fr.end)
    return size, meta, shown


def decode(data):
    """first displayed frame: RGB float planes, alpha plane or None"""
    size, meta, shown = decode_frames(data, True)
    rgb, alpha, _ = shown[0]
    return size, meta, rgb, alpha


def to_rgba8(size, rgb, alpha):
    w, h = size.xsize, size.ysize
    out = np.empty((h, w, 4), np.uint8)
    for c in range(3):
        v = np.asarray(rgb[c], np.float64)[:h, :w]
        out[:, :, c] = np.clip(np.floor(v * 255.0 + 0.5), 0, 255)
    if alpha is None:
        out[:, :, 3] = 255
    else:
        out[:, :, 3] = np.clip(np.floor(np.asarray(alpha, np.float64)[:h, :w] * 255.0 + 0.5), 0, 255)
    return out.tobytes()


# ====================================================================== container / codestream

def codestream(data):
    """the codestream of a bare stream or of the ISOBMFF container (jxlc / jxlp boxes)"""
    if data[:2] == b"\xff\x0a":
        return data
    if data[:12] != b"\x00\x00\x00\x0cJXL \r\n\x87\n":
        raise JXLError("not a JPEG XL file")
    p = 0
    parts = []
    while p + 8 <= len(data):
        size = int.from_bytes(data[p:p + 4], "big")
        typ = data[p + 4:p + 8]
        hdr = 8
        if size == 1:
            size = int.from_bytes(data[p + 8:p + 16], "big")
            hdr = 16
        elif size == 0:
            size = len(data) - p
        body = data[p + hdr:p + size]
        if typ == b"jxlc":
            parts.append(body)
        elif typ == b"jxlp":
            parts.append(body[4:])
        p += size
    if not parts:
        raise JXLError("no codestream box")
    return b"".join(parts)


class Image:
    """decoded result: planes (float or int per channel), metadata"""


def read_headers(cs):
    b = Bits(cs)
    if b.f(16) != 0x0AFF:
        raise JXLError("signature")
    size = SizeHeader().read(b)
    meta = ImageMetadata().read(b)
    td = TransformData().read(b, meta.xyb_encoded)
    icc = read_icc(b) if meta.color.want_icc else None
    b.byte_align()
    return b, size, meta, td, icc


def main():
    import argparse
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("file")
    ap.add_argument("--info", action="store_true")
    ap.add_argument("--rgba", help="write the decoded image as raw RGBA 8-bit")
    a = ap.parse_args()
    if a.rgba:
        size, meta, rgb, alpha = decode(Path(a.file).read_bytes())
        Path(a.rgba).write_bytes(to_rgba8(size, rgb, alpha))
        print("%dx%d" % (size.xsize, size.ysize))
        return
    data = Path(a.file).read_bytes()
    cs = codestream(data)
    b, size, meta, td, icc = read_headers(cs)
    print("size %dx%d bits %d%s xyb %d extra %d color cs %d tf %d icc %s" % (
        size.xsize, size.ysize, meta.bit_depth.bits, "f" if meta.bit_depth.float else "", meta.xyb_encoded,
        len(meta.extra), meta.color.color_space, meta.color.tf, len(icc) if icc else None))
    while True:
        fh = FrameHeader().read(b, meta, size)
        n = toc_entries(fh)
        sizes, offs = read_toc(b, n)
        start = b.p >> 3
        print("frame type %d %s %dx%d passes %d groups %d dc groups %d toc %d bytes %d last %d" % (
            fh.frame_type, "modular" if fh.modular else "vardct", fh.width, fh.height, fh.num_passes,
            fh.num_groups, fh.num_dc_groups, n, sum(sizes), fh.is_last))
        b = Bits(cs, start + sum(sizes))
        if fh.is_last or start + sum(sizes) >= len(cs):
            break


if __name__ == "__main__":
    main()
