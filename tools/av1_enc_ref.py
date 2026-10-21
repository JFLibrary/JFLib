#!/usr/bin/env python3
"""AV1 intra encoder model for the AVIF writer (reference of ASMLIB/src/img/av1_enc.s).

    python tools/av1_enc_ref.py IN.png OUT.obu [--q 0..100] [--chroma 420|444|400] [--bits 8|10]
                                               [--lossless] [--check]

One key frame: reduced still picture header, 64x64 superblocks, uniform tiles (sides of at least
tile_sb superblocks, at most 16 tile columns and 64 tiles, the level limits), square partitions
64 .. 8 chosen by rate-distortion cost (forced splits at the frame edges), intra modes DC .. PAETH
with angle delta 0 for luma and chroma (one transform block: SATD pre-selection of NCAND modes),
TX_MODE_LARGEST (one transform size per block and plane), reduced transform set, luma transform
type up to 16x16 from TX_SEARCH by J (chroma: the type its mode implies), deblocking with a level
from the quantizer (libaom's formula), no CDEF / loop restoration.  Lossless (base_q_idx 0): 4x4
Walsh-Hadamard (libaom's av1_fwht4x4).

Forward transforms: the decoder's butterfly sequences (tools/av1_ref.py) run backwards with the
negated angles (DCT / ADST8 / ADST16; inverse(forward(x)) = N / 2 * x), ADST4 as the transpose of
the inverse's integer matrix, identity with the inverse's factors.  Quantization: A = |C| * K[ txSz ]
with K = 2^( rowShift + 4 ) * dqDenom / ( w / 2 * h / 2 ) (* sqrt( 2 ) for 2:1 sizes) in Q16 (the
dequantized value times 2^16); rounded levels, then RDOQ (rdoq): in reverse scan order l or l - 1
by J with the contexts of the levels chosen so far, trailing ones dropped while J decreases, all
zero when cheaper.  Distortion: the pixel-domain error of a level is ( A / 2^16 - l * q ) / 8 for
every transform size.  Rate: Q8 bits (COST) from the CDFs in effect (no adaptation inside the
search of a superblock); J = SSD * 65536 + lambda * bits with libaom's key frame lambda
q^2 * ( 3.3 + 0.0015 * q8 ) / 2048 per bit (lam_q8), q = dc_q( base_q_idx ).

The coding state is an av1_ref.Decoder: the headers this script writes are parsed by it, and its
prediction, reconstruction and context functions are used, so every stream decodes by
construction (--check decodes the result with av1_ref and compares the reconstruction).
"""

import sys
from math import isqrt
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import av1_ref as R  # noqa: E402
from av1_ref import Round2, Clip3, FloorLog2  # noqa: E402

C = R.T.CONST
globals().update(C)
for _k in R.T.DIMS:
    globals()[_k] = getattr(R.T, _k)
Block_Width = R.Block_Width
Block_Height = R.Block_Height
TX_SET_DCTONLY = 0

# ====================================================================== tables

LEVELS = [  # (seq_level_idx, MaxPicSize, MaxHSize, MaxVSize)
    (0, 147456, 2048, 1152), (1, 278784, 2816, 1584), (4, 665856, 4352, 2448), (5, 1065024, 5504, 3096),
    (8, 2359296, 6144, 3456), (12, 8912896, 8192, 4352), (16, 35651584, 16384, 8704)]

# Q8 bits of a symbol with probability p (Q15): COST[ min( p >> 5, 1023 ) ]
COST = [max(1, round(-__import__("math").log2((i + 0.5) / 1024.0) * 256)) for i in range(1024)]

# luma transform types tried for transform sizes up to 16x16 (reduced set TX_SET_INTRA_2)
TX_SEARCH = (DCT_DCT, ADST_DCT, DCT_ADST, ADST_ADST, IDTX)


def quality_qidx(q, lossless=False):
    """quality 0..100 -> base_q_idx (libavif's quantizer mapping, libaom's quantizer_to_qindex)"""
    if lossless:
        return 0
    quantizer = ((100 - max(0, min(100, q))) * 63 + 50) // 100
    if quantizer == 63:
        return 255
    if quantizer == 62:
        return 249
    return max(1, 4 * quantizer)


def lf_level(qidx, bd):
    """deblocking level of a key frame from the quantizer (libaom LPF_PICK_FROM_Q)"""
    q = Ac_Qlookup[(bd - 8) >> 1][qidx]
    if bd == 8:
        g = (q * 17563 - 421574 + (1 << 17)) >> 18
    elif bd == 10:
        g = ((q * 20723 + 4060632 + (1 << 19)) >> 20) - 4
    else:
        g = ((q * 20723 + 16242526 + (1 << 21)) >> 22) - 4
    return Clip3(0, 63, g)


def lam_q8(qidx, bd):
    """lambda for J = SSD * 65536 + lam * bits (Q8): libaom's key frame rdmult, i.e. SSD per bit
    q^2 * ( 3.3 + 0.0015 * q8 ) / 2048 (q = dc_q at the bit depth, q8 = q at 8 bits), times 256"""
    q = Dc_Qlookup[(bd - 8) >> 1][qidx]
    q8 = q >> (bd - 8)
    return max(1, (q * q * (6600 + 3 * q8)) // 16000)


def kq16(txSz):
    """quantizer scale K (Q16) of a transform size, see the module docstring"""
    import math
    lw, lh = Tx_Width_Log2[txSz], Tx_Height_Log2[txSz]
    w, h = 1 << lw, 1 << lh
    rs = Transform_Row_Shift[txSz]
    den = 4 if txSz in (TX_64X64, TX_32X64, TX_64X32) else 2 if txSz in (TX_32X32, TX_16X32, TX_32X16, TX_16X64,
                                                                           TX_64X16) else 1
    s = (2.0 ** (rs + 4)) * den / ((w / 2.0) * (h / 2.0))
    if abs(lw - lh) == 1:
        s *= math.sqrt(2.0)
    return int(round(s * 65536))


KQ16 = [kq16(t) for t in range(TX_SIZES_ALL)]


# ====================================================================== forward transforms

def record_ops(fn, n):
    ops = []
    oB, oH = R._B, R._H

    def B(T_, a, b, angle, flip):
        ops.append((0, a, b, angle, flip))
        oB(T_, a, b, angle, flip)

    def H(T_, a, b, flip, r):
        if flip:
            a, b = b, a
        ops.append((1, a, b, 0, 0))
        oH(T_, a, b, 0, r)
    R._B, R._H = B, H
    try:
        T_ = [0] * (1 << n)
        if fn == "idct":
            R.idct(T_, n, 30)
        else:
            getattr(R, fn)(T_, 30)
    finally:
        R._B, R._H = oB, oH
    return ops


FWD = {}
for _n in range(2, 7):
    FWD[("dct", _n)] = list(reversed(record_ops("idct", _n)))
FWD[("adst", 3)] = list(reversed(record_ops("iadst8", 3)))
FWD[("adst", 4)] = list(reversed(record_ops("iadst16", 4)))
_S1, _S2, _S3, _S4 = SINPI_1_9, SINPI_2_9, SINPI_3_9, SINPI_4_9
ADST4_INV = [[_S1, _S3, _S4, _S2], [_S2, _S3, -_S1, -_S4], [_S3, 0, -_S3, _S3], [_S4, -_S3, _S2, -_S1]]


def run_fwd_ops(T_, ops):
    for kind, a, b, angle, flip in ops:
        if kind == 1:
            u, v = T_[a], T_[b]
            T_[a] = u + v
            T_[b] = u - v
        else:
            if flip:
                T_[a], T_[b] = T_[b], T_[a]
            ang = (-angle) & 255
            c, s = R.cos128(ang), R.sin128(ang)
            u, v = T_[a], T_[b]
            T_[a] = (u * c - v * s + 2048) >> 12
            T_[b] = (u * s + v * c + 2048) >> 12


def fdct(x, n):
    T_ = list(x)
    run_fwd_ops(T_, FWD[("dct", n)])
    out = [0] * (1 << n)
    for i in range(1 << n):
        out[R.brev(n, i)] = T_[i]
    return out


def fadst(x, n):
    N = 1 << n
    if n == 2:
        return [(sum(ADST4_INV[k][j] * x[k] for k in range(4)) + 2048) >> 12 for j in range(4)]
    outp = []
    for i in range(N):
        a = (i >> 3) & 1
        b = ((i >> 2) & 1) ^ ((i >> 3) & 1)
        c = ((i >> 1) & 1) ^ ((i >> 2) & 1)
        d = (i & 1) ^ ((i >> 1) & 1)
        outp.append(((d << 3) | (c << 2) | (b << 1) | a) >> (4 - n))
    T_ = [0] * N
    for i in range(N):
        T_[outp[i]] = -x[i] if (i & 1) else x[i]
    run_fwd_ops(T_, FWD[("adst", n)])
    out = [0] * N
    for i in range(N):
        out[(i - 1) if (i & 1) else (N - i - 1)] = T_[i]
    return out


def fidentity(x, n):
    """forward identity = the inverse's scaling (inverse( forward( x ) ) = N / 2 * x)"""
    if n == 2:
        return [(v * 5793 + 2048) >> 12 for v in x]
    if n == 3:
        return [v * 2 for v in x]
    if n == 4:
        return [(v * 11586 + 2048) >> 12 for v in x]
    return [v * 4 for v in x]


# 1-D kinds of a transform type: 0 DCT, 1 ADST, 2 identity (the decoder's rowDct / rowAdst / colDct ...)
ROW_KIND = {DCT_DCT: 0, ADST_DCT: 0, DCT_ADST: 1, ADST_ADST: 1, IDTX: 2}
COL_KIND = {DCT_DCT: 0, DCT_ADST: 0, ADST_DCT: 1, ADST_ADST: 1, IDTX: 2}
FWD1 = (fdct, fadst, fidentity)


def fwd2d(res, txSz, txType):
    """forward transform of the residual res[ h ][ w ] -> coefficients C[ i ][ j ] (rows first,
    exact integers: no intermediate rounding besides the butterflies' Round2( ., 12 ))"""
    lw, lh = Tx_Width_Log2[txSz], Tx_Height_Log2[txSz]
    w, h = 1 << lw, 1 << lh
    fr, fc = FWD1[ROW_KIND[txType]], FWD1[COL_KIND[txType]]
    rows = [fr(res[i], lw) for i in range(h)]
    out = [[0] * w for _ in range(h)]
    for j in range(w):
        col = fc([rows[i][j] for i in range(h)], lh)
        for i in range(h):
            out[i][j] = col[i]
    return out


def fwht4x4(res):
    """libaom av1_fwht4x4 (columns, then rows; outputs times UNIT_QUANT_FACTOR = 4)"""
    tmp = [[0] * 4 for _ in range(4)]
    for i in range(4):
        a1, b1, c1, d1 = res[0][i], res[1][i], res[2][i], res[3][i]
        a1 += b1
        d1 = d1 - c1
        e1 = (a1 - d1) >> 1
        b1 = e1 - b1
        c1 = e1 - c1
        a1 -= c1
        d1 += b1
        tmp[0][i], tmp[1][i], tmp[2][i], tmp[3][i] = a1, c1, d1, b1
    out = [[0] * 4 for _ in range(4)]
    for i in range(4):
        a1, b1, c1, d1 = tmp[i]
        a1 += b1
        d1 -= c1
        e1 = (a1 - d1) >> 1
        b1 = e1 - b1
        c1 = e1 - c1
        a1 -= c1
        d1 += b1
        out[i] = [a1 * 4, c1 * 4, d1 * 4, b1 * 4]
    return out


# ====================================================================== entropy coding

class ECEnc:
    """range encoder (libaom od_ec_enc): the inverse of the specification's symbol decoder"""

    def __init__(self):
        self.low = 0
        self.rng = 0x8000
        self.cnt = -9
        self.pre = []

    def _norm(self, low, rng):
        d = 16 - rng.bit_length()
        c = self.cnt
        s = c + d
        if s >= 0:
            c += 16
            m = (1 << c) - 1
            if s >= 8:
                self.pre.append((low >> c) & 0xFFFF)
                low &= m
                c -= 8
                m >>= 8
            self.pre.append((low >> c) & 0xFFFF)
            s = c + d - 24
            low &= m
        self.low = low << d
        self.rng = rng << d
        self.cnt = s

    def encode(self, s, cdf, n, adapt=True):
        fl = 32768 - (cdf[s - 1] if s > 0 else 0)
        fh = 32768 - cdf[s]
        r = self.rng
        low = self.low
        N = n - 1
        if fl < 32768:
            u = ((r >> 8) * (fl >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (N - (s - 1))
            v = ((r >> 8) * (fh >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (N - s)
            low += r - u
            r = u - v
        else:
            r -= ((r >> 8) * (fh >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (N - s)
        self._norm(low, r)
        if adapt:
            adapt_cdf(cdf, s, n)

    def bool(self, b):
        self.encode(b, [1 << 14, 1 << 15, 0], 2, False)

    def lit(self, v, nbits):
        for i in range(nbits - 1, -1, -1):
            self.bool((v >> i) & 1)

    def done(self):
        low = self.low
        c = self.cnt
        s = 10
        m = 0x3FFF
        e = ((low + m) & ~m) | (m + 1)
        s += c
        pre = list(self.pre)
        if s > 0:
            n = (1 << (c + 16)) - 1
            while True:
                pre.append((e >> (c + 16)) & 0xFFFF)
                e &= n
                s -= 8
                c -= 8
                n >>= 8
                if s <= 0:
                    break
        out = bytearray(len(pre))
        c = 0
        for i in range(len(pre) - 1, -1, -1):
            c = pre[i] + c
            out[i] = c & 0xFF
            c >>= 8
        return bytes(out)


def adapt_cdf(cdf, s, n):
    """the CDF update of the specification's read_symbol"""
    rate = 3 + (cdf[n] > 15) + (cdf[n] > 31) + min(FloorLog2(n), 2)
    tmp = 0
    for i in range(n - 1):
        if i == s:
            tmp = 1 << 15
        if tmp < cdf[i]:
            cdf[i] -= (cdf[i] - tmp) >> rate
        else:
            cdf[i] += (tmp - cdf[i]) >> rate
    cdf[n] += cdf[n] < 32


def cost(cdf, s):
    """Q8 bits of symbol s"""
    return COST[min((cdf[s] - (cdf[s - 1] if s > 0 else 0)) >> 5, 1023)]


def br_cost(cdf, level):
    """Q8 bits of the coeff_br symbols of a level > NUM_BASE_LEVELS"""
    r = 0
    rem = level - 3
    for _ in range(COEFF_BASE_RANGE // (BR_CDF_SIZE - 1)):
        k = min(rem, 3)
        r += cost(cdf, k)
        rem -= k
        if k < 3:
            break
    return r


def sign_golomb_cost(level, c, dcs):
    """Q8 bits of the sign (scan position c; dcs: the DcSign CDF, cost of the cheaper sign) and of
    the Golomb remainder of a level > 14"""
    r = min(cost(dcs, 0), cost(dcs, 1)) if c == 0 else 256
    if level > NUM_BASE_LEVELS + COEFF_BASE_RANGE:
        r += 256 * (2 * (level - COEFF_BASE_RANGE - NUM_BASE_LEVELS).bit_length() - 1)
    return r


class EncCoder:
    """writes symbols (CDFs adapt)"""

    def __init__(self, ec):
        self.ec = ec
        self.bits = 0

    def sym(self, cdf, s, n):
        self.ec.encode(s, cdf, n)

    def lit(self, v, nbits):
        self.ec.lit(v, nbits)


class EstCoder:
    """sums the Q8 cost of symbols (CDFs unchanged)"""

    def __init__(self):
        self.bits = 0

    def sym(self, cdf, s, n):
        self.bits += cost(cdf, s)

    def lit(self, v, nbits):
        self.bits += 256 * nbits


class BitW:
    def __init__(self):
        self.bits = []

    def f(self, v, n):
        for i in range(n - 1, -1, -1):
            self.bits.append((v >> i) & 1)

    def align(self):
        while len(self.bits) & 7:
            self.bits.append(0)

    def bytes(self):
        b = list(self.bits)
        while len(b) & 7:
            b.append(0)
        return bytes(int("".join(map(str, b[i:i + 8])), 2) for i in range(0, len(b), 8))


def leb128(v):
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def obu(typ, payload):
    return bytes([(typ << 3) | 2]) + leb128(len(payload)) + payload


# ====================================================================== encoder

class Encoder:
    def __init__(self, planes, w, h, chroma, bd, qidx, lf=True, cicp=(1, 13, 1, 1), tile_sb=4):
        """planes: list of row lists (u16), chroma 0 (4:0:0) / 1 (4:2:0) / 3 (4:4:4)"""
        self.src = planes
        self.tile_sb = tile_sb
        self.w, self.h, self.chroma, self.bd, self.qidx = w, h, chroma, bd, qidx
        self.lossless = qidx == 0
        self.lf = lf and not self.lossless
        self.cicp = cicp
        self.lam = lam_q8(qidx, bd) if qidx else 256    # lossless: no distortion, J = rate
        self.lam_satd = 16 * isqrt(self.lam)
        self.stats = {}

    # ------------------------------------------------------------------ headers
    def seq_header(self):
        w, h, bd, ch = self.w, self.h, self.bd, self.chroma
        b = BitW()
        profile = 1 if ch == 3 else 0
        b.f(profile, 3)
        b.f(1, 1)               # still_picture
        b.f(1, 1)               # reduced_still_picture_header
        lvl = 31
        for idx, mps, mh, mv in LEVELS:
            if w * h <= mps and w <= mh and h <= mv:
                lvl = idx
                break
        b.f(lvl, 5)
        wb = max(1, (w - 1).bit_length())
        hb = max(1, (h - 1).bit_length())
        b.f(wb - 1, 4)
        b.f(hb - 1, 4)
        b.f(w - 1, wb)
        b.f(h - 1, hb)
        b.f(0, 1)               # use_128x128_superblock
        b.f(0, 1)               # enable_filter_intra
        b.f(1, 1)               # enable_intra_edge_filter
        b.f(0, 1)               # enable_superres
        b.f(0, 1)               # enable_cdef
        b.f(0, 1)               # enable_restoration
        # color_config
        b.f(1 if bd > 8 else 0, 1)      # high_bitdepth (profiles 0 / 1: 8 or 10 bits)
        if profile != 1:
            b.f(1 if ch == 0 else 0, 1)  # mono_chrome
        b.f(1, 1)               # color_description_present_flag
        p, t, m, full = self.cicp
        b.f(p, 8)
        b.f(t, 8)
        b.f(m, 8)
        if ch == 0:
            b.f(full, 1)
        elif not (p == 1 and t == 13 and m == 0):
            b.f(full, 1)        # color_range
            if ch == 1:
                b.f(0, 2)       # chroma_sample_position: unknown
        if ch != 0:
            b.f(0, 1)           # separate_uv_delta_q
        b.f(0, 1)               # film_grain_params_present
        b.f(1, 1)               # trailing_bits
        return b.bytes()

    def frame_header(self, d):
        """uncompressed_header bits (key frame, reduced still picture header), given the decoder
        state d after the sequence header"""
        b = BitW()
        b.f(0, 1)               # disable_cdf_update
        b.f(0, 1)               # allow_screen_content_tools (seq_force = SELECT)
        b.f(0, 1)               # render_and_frame_size_different
        # tile_info: uniform spacing, tiles of at least tile_sb superblocks per side (threads),
        # at most 16 tile columns and 64 tiles
        miCols = 2 * ((self.w + 7) >> 3)
        miRows = 2 * ((self.h + 7) >> 3)
        sbCols = (miCols + 15) >> 4
        sbRows = (miRows + 15) >> 4
        maxTileWidthSb = MAX_TILE_WIDTH >> 6
        maxTileAreaSb = MAX_TILE_AREA >> 12
        minLog2TileCols = R.tile_log2(maxTileWidthSb, sbCols)
        maxLog2TileCols = R.tile_log2(1, min(sbCols, MAX_TILE_COLS))
        maxLog2TileRows = R.tile_log2(1, min(sbRows, MAX_TILE_ROWS))
        minLog2Tiles = max(minLog2TileCols, R.tile_log2(maxTileAreaSb, sbRows * sbCols))
        b.f(1, 1)               # uniform_tile_spacing_flag
        tcl = Clip3(minLog2TileCols, min(maxLog2TileCols, 4), FloorLog2(max(1, sbCols // self.tile_sb)))
        for _ in range(minLog2TileCols, tcl):
            b.f(1, 1)           # increment_tile_cols_log2
        if tcl < maxLog2TileCols:
            b.f(0, 1)
        minLog2TileRows = max(minLog2Tiles - tcl, 0)
        trl = Clip3(minLog2TileRows, max(minLog2TileRows, min(maxLog2TileRows, 6 - tcl)),
                    FloorLog2(max(1, sbRows // self.tile_sb)))
        for _ in range(minLog2TileRows, trl):
            b.f(1, 1)           # increment_tile_rows_log2
        if trl < maxLog2TileRows:
            b.f(0, 1)
        if tcl or trl:
            b.f(0, tcl + trl)   # context_update_tile_id
            b.f(3, 2)           # tile_size_bytes_minus_1: 4 bytes
        # quantization_params
        b.f(self.qidx, 8)
        b.f(0, 1)               # DeltaQYDc
        if self.chroma:
            b.f(0, 1)           # DeltaQUDc
            b.f(0, 1)           # DeltaQUAc
        b.f(0, 1)               # using_qmatrix
        b.f(0, 1)               # segmentation_enabled
        if self.qidx > 0:
            b.f(0, 1)           # delta_q_present
        if not self.lossless:
            lvl = lf_level(self.qidx, self.bd) if self.lf else 0
            b.f(lvl, 6)
            b.f(lvl, 6)
            if self.chroma and lvl:
                b.f(lvl, 6)
                b.f(lvl, 6)
            b.f(0, 3)           # loop_filter_sharpness
            b.f(0, 1)           # loop_filter_delta_enabled
            b.f(0, 1)           # tx_mode_select = 0: TX_MODE_LARGEST
        b.f(1, 1)               # reduced_tx_set
        return b

    # ------------------------------------------------------------------ top level
    def encode(self):
        seq = self.seq_header()
        d = R.Decoder(True)
        d.decode(obu(OBU_SEQUENCE_HEADER, seq))
        fh = self.frame_header(d)
        hb = fh.bytes()
        d.SeenFrameHeader = 0
        d.frame_header_obu(R.Bits(hb))
        self.d = d
        # tiles
        tiles = []
        for tr in range(d.TileRows):
            for tc in range(d.TileCols):
                tiles.append(self.encode_tile(tr, tc))
        # OBU_FRAME: header, byte alignment, tile group
        b = fh
        b.align()
        if len(tiles) > 1:
            b.f(0, 1)           # tile_start_and_end_present_flag
            b.align()
        body = b.bytes()
        for i, t in enumerate(tiles):
            if i < len(tiles) - 1:
                body += (len(t) - 1).to_bytes(4, "little")
            body += t
        self.recon = d.CurrFrame
        return obu(OBU_SEQUENCE_HEADER, seq) + obu(OBU_FRAME, body)

    def encode_tile(self, tr, tc):
        d = self.d
        d.MiRowStart = d.MiRowStarts[tr]
        d.MiRowEnd = d.MiRowStarts[tr + 1]
        d.MiColStart = d.MiColStarts[tc]
        d.MiColEnd = d.MiColStarts[tc + 1]
        d.CurrentQIndex = d.base_q_idx
        d.cdf = {k: R.cdf_copy(v) for k, v in d.fcdf.items()}
        d.cdf["IntraFrameYMode"] = R.cdf_copy(Default_Intra_Frame_Y_Mode_Cdf)
        W = d.MiCols + 40
        H = d.MiRows + 40
        d.AboveLevelContext = [[0] * W for _ in range(3)]
        d.AboveDcContext = [[0] * W for _ in range(3)]
        d.AboveSegPredContext = [0] * W
        d.DeltaLF = [0] * FRAME_LF_COUNT
        ec = ECEnc()
        self.enc = EncCoder(ec)
        for r in range(d.MiRowStart, d.MiRowEnd, 16):
            d.LeftLevelContext = [[0] * H for _ in range(3)]
            d.LeftDcContext = [[0] * H for _ in range(3)]
            d.LeftSegPredContext = [0] * H
            for c in range(d.MiColStart, d.MiColEnd, 16):
                d.ReadDeltas = 0
                d.clear_cdef(r, c)
                d.clear_block_decoded_flags(r, c, 16)
                s0 = self.snap(r, c, BLOCK_64X64)
                tree = self.search(r, c, BLOCK_64X64)[1]
                self.restore(s0)
                self.emit(r, c, BLOCK_64X64, tree)
        return ec.done()

    # ------------------------------------------------------------------ state snapshots
    MI_ARRAYS = ("YModes", "UVModes", "Skips", "TxSizes", "InterTxSizes", "MiSizes", "SegmentIds", "TxTypes")

    def snap(self, r, c, bSize):
        d = self.d
        bw4, bh4 = Num_4x4_Blocks_Wide[bSize], Num_4x4_Blocks_High[bSize]
        st = {}
        for name in self.MI_ARRAYS:
            a = getattr(d, name)
            st[name] = [list(a[y][c:c + bw4]) for y in range(r, r + bh4)]
        st["PaletteSizes"] = [[list(a[y][c:c + bw4]) for y in range(r, r + bh4)] for a in d.PaletteSizes]
        st["PaletteColors"] = [[list(a[y][c:c + bw4]) for y in range(r, r + bh4)] for a in d.PaletteColors]
        st["DeltaLFs"] = [list(d.DeltaLFs[y][c:c + bw4]) for y in range(r, r + bh4)]
        frames = []
        ltx = []
        for p in range(d.NumPlanes):
            sx = d.subsampling_x if p else 0
            sy = d.subsampling_y if p else 0
            x0, y0 = (c * 4) >> sx, (r * 4) >> sy
            pw, ph = max(4, (bw4 * 4) >> sx), max(4, (bh4 * 4) >> sy)
            frames.append([list(d.CurrFrame[p][y][x0:x0 + pw]) for y in range(y0, y0 + ph)])
            r4, c4 = r >> sy, c >> sx
            ltx.append([list(d.LoopfilterTxSizes[p][y][c4:c4 + max(1, bw4 >> sx)])
                        for y in range(r4, r4 + max(1, bh4 >> sy))])
        st["frames"] = frames
        st["ltx"] = ltx
        st["above"] = ([list(a) for a in d.AboveLevelContext], [list(a) for a in d.AboveDcContext])
        st["left"] = ([list(a) for a in d.LeftLevelContext], [list(a) for a in d.LeftDcContext])
        st["bd"] = [[list(row) for row in bdp] for bdp in d.BlockDecoded]
        return (r, c, bSize, st)

    def restore(self, snapshot):
        r, c, bSize, st = snapshot
        d = self.d
        bw4, bh4 = Num_4x4_Blocks_Wide[bSize], Num_4x4_Blocks_High[bSize]
        for name in self.MI_ARRAYS:
            a = getattr(d, name)
            for i, y in enumerate(range(r, r + bh4)):
                a[y][c:c + bw4] = st[name][i]
        for k in range(2):
            for i, y in enumerate(range(r, r + bh4)):
                d.PaletteSizes[k][y][c:c + bw4] = st["PaletteSizes"][k][i]
                d.PaletteColors[k][y][c:c + bw4] = st["PaletteColors"][k][i]
        for i, y in enumerate(range(r, r + bh4)):
            d.DeltaLFs[y][c:c + bw4] = st["DeltaLFs"][i]
        for p in range(d.NumPlanes):
            sx = d.subsampling_x if p else 0
            sy = d.subsampling_y if p else 0
            x0, y0 = (c * 4) >> sx, (r * 4) >> sy
            for i, row in enumerate(st["frames"][p]):
                d.CurrFrame[p][y0 + i][x0:x0 + len(row)] = row
            r4, c4 = r >> sy, c >> sx
            for i, row in enumerate(st["ltx"][p]):
                d.LoopfilterTxSizes[p][r4 + i][c4:c4 + len(row)] = row
        d.AboveLevelContext = [list(a) for a in st["above"][0]]
        d.AboveDcContext = [list(a) for a in st["above"][1]]
        d.LeftLevelContext = [list(a) for a in st["left"][0]]
        d.LeftDcContext = [list(a) for a in st["left"][1]]
        d.BlockDecoded = [[list(row) for row in bdp] for bdp in st["bd"]]

    # ------------------------------------------------------------------ partition search
    def part_cost(self, r, c, bSize, partition):
        """Q8 bits of coding partition (or the split flag at the frame edges)"""
        d = self.d
        if bSize < BLOCK_8X8:
            return 0
        half = Num_4x4_Blocks_Wide[bSize] >> 1
        hasRows = (r + half) < d.MiRows
        hasCols = (c + half) < d.MiCols
        est = EstCoder()
        self.code_partition(est, r, c, bSize, partition, hasRows, hasCols)
        return est.bits

    def code_partition(self, cw, r, c, bSize, partition, hasRows, hasCols):
        d = self.d
        AvailU = d.is_inside(r - 1, c)
        AvailL = d.is_inside(r, c - 1)
        if hasRows and hasCols:
            cdf = d.partition_cdf(r, c, bSize, AvailU, AvailL)
            n = len(cdf) - 1
            cw.sym(cdf, partition, n)
        elif hasCols or hasRows:
            pc = d.partition_cdf(r, c, bSize, AvailU, AvailL)
            if hasCols:
                psum = (pc[2] - pc[1] + pc[3] - pc[2] + pc[4] - pc[3] + pc[6] - pc[5] + pc[7] - pc[6])
                if bSize != BLOCK_128X128:
                    psum += pc[9] - pc[8]
            else:
                psum = (pc[1] - pc[0] + pc[3] - pc[2] + pc[4] - pc[3] + pc[5] - pc[4] + pc[6] - pc[5])
                if bSize != BLOCK_128X128:
                    psum += pc[8] - pc[7]
            cw.sym([(1 << 15) - psum, 1 << 15, 0], 1 if partition == PARTITION_SPLIT else 0, 2)

    def search(self, r, c, bSize):
        """-> (J, tree): tree = ("none", decisions) / ("split", [4 subtrees]) / None (outside)"""
        d = self.d
        if r >= d.MiRows or c >= d.MiCols:
            return 0, None
        half = Num_4x4_Blocks_Wide[bSize] >> 1
        hasRows = (r + half) < d.MiRows
        hasCols = (c + half) < d.MiCols
        can_none = bSize == BLOCK_8X8 or (hasRows and hasCols)
        can_split = bSize > BLOCK_8X8
        best = None
        if can_none:
            s0 = self.snap(r, c, bSize)
            J, dec = self.block(r, c, bSize, EstCoder())
            J += self.lam * self.part_cost(r, c, bSize, PARTITION_NONE)
            best = (J, ("none", dec))
            if can_split:
                s1 = self.snap(r, c, bSize)
                self.restore(s0)
        if can_split:
            sub = Partition_Subsize[PARTITION_SPLIT][bSize]
            Js = self.lam * self.part_cost(r, c, bSize, PARTITION_SPLIT)
            subs = []
            for (dr, dc) in ((0, 0), (0, half), (half, 0), (half, half)):
                j, t = self.search(r + dr, c + dc, sub)
                Js += j
                subs.append(t)
            if best is None or Js < best[0]:
                best = (Js, ("split", subs))
            else:
                self.restore(s1)
        return best

    def emit(self, r, c, bSize, tree):
        d = self.d
        if tree is None:
            return
        half = Num_4x4_Blocks_Wide[bSize] >> 1
        hasRows = (r + half) < d.MiRows
        hasCols = (c + half) < d.MiCols
        kind = tree[0]
        part = PARTITION_NONE if kind == "none" else PARTITION_SPLIT
        if bSize >= BLOCK_8X8:
            self.code_partition(self.enc, r, c, bSize, part, hasRows, hasCols)
        if kind == "none":
            self.block(r, c, bSize, self.enc, tree[1])
        else:
            sub = Partition_Subsize[PARTITION_SPLIT][bSize]
            for k, (dr, dc) in enumerate(((0, 0), (0, half), (half, 0), (half, half))):
                self.emit(r + dr, c + dc, sub, tree[1][k])

    # ------------------------------------------------------------------ blocks
    def set_block(self, r, c, bSize):
        d = self.d
        d.MiRow, d.MiCol, d.MiSize = r, c, bSize
        bw4, bh4 = Num_4x4_Blocks_Wide[bSize], Num_4x4_Blocks_High[bSize]
        ssx, ssy = d.subsampling_x, d.subsampling_y
        if bh4 == 1 and ssy and (r & 1) == 0:
            d.HasChroma = 0
        elif bw4 == 1 and ssx and (c & 1) == 0:
            d.HasChroma = 0
        else:
            d.HasChroma = int(d.NumPlanes > 1)
        d.AvailU = d.is_inside(r - 1, c)
        d.AvailL = d.is_inside(r, c - 1)
        d.AvailUChroma = d.AvailU
        d.AvailLChroma = d.AvailL
        if d.HasChroma:
            if ssy and bh4 == 1:
                d.AvailUChroma = d.is_inside(r - 2, c)
            if ssx and bw4 == 1:
                d.AvailLChroma = d.is_inside(r, c - 2)
        else:
            d.AvailUChroma = d.AvailLChroma = 0
        d.segment_id = 0
        d.Lossless = d.LosslessArray[0]
        d.use_filter_intra = 0
        d.PaletteSizeY = d.PaletteSizeUV = 0
        d.palette_colors_y = d.palette_colors_u = d.palette_colors_v = None
        d.AngleDeltaY = d.AngleDeltaUV = 0
        d.CflAlphaU = d.CflAlphaV = 0
        d.use_intrabc = 0
        d.is_inter = 0
        d.TxSize = TX_4X4 if d.Lossless else Max_Tx_Size_Rect[bSize]

    def plane_geom(self, plane):
        d = self.d
        sx = d.subsampling_x if plane else 0
        sy = d.subsampling_y if plane else 0
        txSz = TX_4X4 if d.Lossless else d.get_tx_size(plane, d.TxSize)
        x0 = (d.MiCol >> sx) * 4
        y0 = (d.MiRow >> sy) * 4
        planeSz = d.get_plane_residual_size(d.MiSize, plane)
        return sx, sy, txSz, x0, y0, Block_Width[planeSz], Block_Height[planeSz]

    def tx_blocks(self, plane):
        """[(startX, startY, x4 offset, y4 offset)] of the plane's transform blocks inside the frame"""
        d = self.d
        sx, sy, txSz, x0, y0, bw, bh = self.plane_geom(plane)
        tw, th = Tx_Width[txSz], Tx_Height[txSz]
        maxX = (d.MiCols * MI_SIZE) >> sx
        maxY = (d.MiRows * MI_SIZE) >> sy
        out = []
        for y in range(0, bh, th):
            for x in range(0, bw, tw):
                if x0 + x < maxX and y0 + y < maxY:
                    out.append((x0 + x, y0 + y, x >> 2, y >> 2))
        return txSz, out

    def predict(self, plane, mode, sx_, sy_, txSz, bx4, by4):
        """predict_intra of one transform block (as transform_block does)"""
        d = self.d
        sx = d.subsampling_x if plane else 0
        sy = d.subsampling_y if plane else 0
        row = (sy_ << sy) >> 2
        col = (sx_ << sx) >> 2
        sbr, sbc = row & 15, col & 15
        stepX, stepY = Tx_Width[txSz] >> 2, Tx_Height[txSz] >> 2
        isCfl = plane > 0 and mode == UV_CFL_PRED
        m = DC_PRED if isCfl else mode
        d.predict_intra(plane, sx_, sy_,
                        (d.AvailL if plane == 0 else d.AvailLChroma) or bx4 > 0,
                        (d.AvailU if plane == 0 else d.AvailUChroma) or by4 > 0,
                        d.bdec(plane, (sbr >> sy) - 1, (sbc >> sx) + stepX),
                        d.bdec(plane, (sbr >> sy) + stepY, (sbc >> sx) - 1),
                        m, Tx_Width_Log2[txSz], Tx_Height_Log2[txSz])
        if isCfl:
            d.predict_chroma_from_luma(plane, sx_, sy_, txSz)

    def residual_of(self, plane, x, y, txSz):
        d = self.d
        tw, th = Tx_Width[txSz], Tx_Height[txSz]
        src = self.src[plane]
        F = d.CurrFrame[plane]
        ph, pw = len(src), len(src[0])
        res = []
        for i in range(th):
            sy = min(y + i, ph - 1)
            srow = src[sy]
            frow = F[y + i]
            res.append([srow[min(x + j, pw - 1)] - frow[x + j] for j in range(tw)])
        return res

    def satd(self, res):
        """sum of absolute 4x4 Hadamard coefficients"""
        h, w = len(res), len(res[0])
        t = 0
        for y in range(0, h, 4):
            for x in range(0, w, 4):
                m = [[res[y + i][x + j] for j in range(4)] for i in range(4)]
                for i in range(4):
                    a, b, c, e = m[i]
                    s0, s1, d0, d1 = a + b, c + e, a - b, c - e
                    m[i] = [s0 + s1, s0 - s1, d0 + d1, d0 - d1]
                for j in range(4):
                    a, b, c, e = m[0][j], m[1][j], m[2][j], m[3][j]
                    s0, s1, d0, d1 = a + b, c + e, a - b, c - e
                    t += abs(s0 + s1) + abs(s0 - s1) + abs(d0 + d1) + abs(d0 - d1)
        return t

    def quant_txb(self, plane, x, y, txSz, txType, res):
        """forward transform + quantization (RDOQ) of one transform block;
        -> (levels: signed, pos = i * tw + j of the coded tw x th area, transform-domain distortion
        in J units)

        A = |C| * K is the coefficient in Q16 units of q (the dequantized value times dqDenom);
        the pixel-domain error of a level l is ( A / 2^16 - l * q ) / 8 for every transform size
        (AV1's scaling), so its J = 65536 * SSD is ( ( A >> 8 ) - l * q * 256 )^2 >> 6."""
        d = self.d
        lw, lh = Tx_Width_Log2[txSz], Tx_Height_Log2[txSz]
        w, h = 1 << lw, 1 << lh
        tw, th = min(32, w), min(32, h)
        if d.Lossless:
            C_ = fwht4x4(res)
            return [C_[i][j] >> 2 if C_[i][j] >= 0 else -((-C_[i][j]) >> 2) for i in range(4) for j in range(4)], 0
        C_ = fwd2d(res, txSz, txType)
        qi = d.get_qindex(0, 0)
        dcq = d.dc_q(qi + (d.DeltaQYDc, d.DeltaQUDc, d.DeltaQVDc)[plane])
        acq = d.ac_q(qi + (0, d.DeltaQUAc, d.DeltaQVAc)[plane])
        K = KQ16[txSz]
        n = tw * th
        A = [0] * n
        neg = [False] * n
        lv = [0] * n
        dz = 0          # J of the coefficients outside the coded area (64-point transforms)
        for i in range(h):
            row = C_[i]
            for j in range(w):
                v = row[j]
                if i >= 32 or j >= 32:
                    a8 = (abs(v) * K) >> 8
                    dz += (a8 * a8) >> 6
                    continue
                pos = i * tw + j
                q = dcq if pos == 0 else acq
                A[pos] = abs(v) * K
                neg[pos] = v < 0
                lv[pos] = (A[pos] + (q << 15)) // (q << 16)
        d.PlaneTxType = txType if plane == 0 else d.compute_tx_type(plane, txSz, 0, 0)
        out = self.rdoq(plane, x, y, txSz, txType, A, lv, dcq, acq)
        dist = dz
        for pos in range(n):
            q = dcq if pos == 0 else acq
            e = (A[pos] >> 8) - out[pos] * q * 256
            dist += (e * e) >> 6
            if neg[pos] and out[pos]:
                out[pos] = -out[pos]
        return out, dist

    def rdoq(self, plane, x, y, txSz, txType, A, lv, dcq, acq):
        """levels (unsigned) from the rounded levels lv: in reverse scan order every level l > 0
        becomes l or l - 1, whichever has the smaller J with the contexts of the levels chosen so
        far (the last one stays nonzero); then trailing ones are dropped while J decreases, and the
        block becomes all zero when that is cheaper"""
        d = self.d
        scan = d.get_scan(txSz)
        eob = 0
        for c in range(len(scan) - 1, -1, -1):
            if lv[scan[c]]:
                eob = c + 1
                break
        n = len(A)
        if eob == 0:
            return [0] * n
        lam = self.lam
        txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1
        ptype = int(plane > 0)
        adjTxSz = Adjusted_Tx_Size[txSz]
        bwl = Tx_Width_Log2[adjTxSz]
        width = 1 << bwl
        height = Tx_Height[adjTxSz]
        txClass = TX_CLASS_2D
        cb = d.cdf["CoeffBase"][txSzCtx][ptype]
        cbe = d.cdf["CoeffBaseEob"][txSzCtx][ptype]
        cbr = d.cdf["CoeffBr"][min(txSzCtx, TX_32X32)][ptype]
        x4, y4 = x >> 2, y >> 2
        w4, h4 = Tx_Width[txSz] >> 2, Tx_Height[txSz] >> 2
        dcs = d.cdf["DcSign"][ptype][d.dc_sign_ctx(plane, x4, y4, w4, h4)]
        Quant = [0] * n
        out = [0] * n
        for c in range(eob - 1, -1, -1):
            pos = scan[c]
            l0 = lv[pos]
            if l0 == 0:
                continue
            q = dcq if pos == 0 else acq
            a8 = A[pos] >> 8
            if c == eob - 1:
                cx = self.eob_ctx(c, bwl, height)
            else:
                cx = d.coeff_base_ctx(txSz, txClass, bwl, width, height, pos, Quant)
            brx = d.coeff_br_ctx(txSz, txClass, pos, Quant)
            best = None
            for l in (l0, l0 - 1):
                if l == 0 and c == eob - 1:
                    continue
                e = a8 - l * q * 256
                r = 0
                if c == eob - 1:
                    r += cost(cbe[cx], min(l, 3) - 1)
                else:
                    r += cost(cb[cx], min(l, 3))
                if l > NUM_BASE_LEVELS:
                    r += br_cost(cbr[brx], l)
                if l:
                    r += sign_golomb_cost(l, c, dcs)
                J = ((e * e) >> 6) + lam * r
                if best is None or J < best[0]:
                    best = (J, l)
            out[pos] = best[1]
            Quant[pos] = min(best[1], 15)

        def jtot(levels):
            dist = 0
            for pos in range(n):
                q = dcq if pos == 0 else acq
                e = (A[pos] >> 8) - levels[pos] * q * 256
                dist += (e * e) >> 6
            return dist + lam * self.txb_rate(plane, x, y, txSz, levels, txType)
        J = jtot(out)
        for _ in range(8):
            last = -1
            for c in range(eob - 1, -1, -1):
                if out[scan[c]]:
                    last = c
                    break
            if last < 0 or out[scan[last]] != 1:
                break
            t = list(out)
            t[scan[last]] = 0
            J2 = jtot(t)
            if J2 >= J:
                break
            out, J = t, J2
        if any(out):
            z = [0] * n
            if jtot(z) < J:
                out = z
        return out

    @staticmethod
    def eob_ctx(c, bwl, height):
        if c == 0:
            cx = SIG_COEF_CONTEXTS - 4
        elif c <= (height << bwl) // 8:
            cx = SIG_COEF_CONTEXTS - 3
        elif c <= (height << bwl) // 4:
            cx = SIG_COEF_CONTEXTS - 2
        else:
            cx = SIG_COEF_CONTEXTS - 1
        return cx - SIG_COEF_CONTEXTS + SIG_COEF_CONTEXTS_EOB

    def txb_rate(self, plane, x, y, txSz, quant, txType):
        """Q8 bits of coding the transform block (all_zero, type, eob, levels, signs)"""
        est = EstCoder()
        self.code_txb(est, plane, x, y, txSz, quant, txType, False)
        return est.bits

    def plane_tx_type(self, plane, txSz):
        """luma: DCT_DCT; chroma: compute_tx_type"""
        d = self.d
        if plane == 0:
            return DCT_DCT
        return d.compute_tx_type(plane, txSz, 0, 0)

    def code_txb(self, cw, plane, x, y, txSz, quant, txType, update=True):
        """coeffs( plane, x, y, txSz ) of the decoder, written: all_zero, (luma) transform type,
        eob, levels, signs, golomb; update: set the contexts and TxTypes like the decoder (else
        the transform type of luma is txType)"""
        d = self.d
        x4, y4 = x >> 2, y >> 2
        w4, h4 = Tx_Width[txSz] >> 2, Tx_Height[txSz] >> 2
        txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1
        ptype = int(plane > 0)
        eob = 0
        culLevel = 0
        dcCategory = 0
        if plane == 0:
            d.PlaneTxType = txType
        nz = any(quant)
        ctx = d.all_zero_ctx(plane, txSz, x4, y4, w4, h4)
        cw.sym(d.cdf["TxbSkip"][txSzCtx][ctx], 0 if nz else 1, 2)
        if not nz:
            if plane == 0 and update:
                for i in range(w4):
                    for j in range(h4):
                        d.TxTypes[y4 + j][x4 + i] = DCT_DCT
        else:
            if plane == 0:
                # transform_type
                s = d.get_tx_set(txSz)
                q = d.base_q_idx
                if s > 0 and q > 0:
                    intraDir = d.YMode
                    if s == TX_SET_INTRA_1:
                        cw.sym(d.cdf["IntraTxTypeSet1"][Tx_Size_Sqr[txSz]][intraDir],
                               Tx_Type_Intra_Inv_Set1.index(txType), 7)
                    else:
                        cw.sym(d.cdf["IntraTxTypeSet2"][Tx_Size_Sqr[txSz]][intraDir],
                               Tx_Type_Intra_Inv_Set2.index(txType), 5)
                if update:
                    for i in range(w4):
                        for j in range(h4):
                            d.TxTypes[y4 + j][x4 + i] = txType
            d.PlaneTxType = txType if plane == 0 else d.compute_tx_type(plane, txSz, x4, y4)
            txClass = R.get_tx_class(d.PlaneTxType)
            scan = d.get_scan(txSz)
            last = max(c for c in range(len(scan)) if scan[c] < len(quant) and quant[scan[c]])
            eob = last + 1
            eobMultisize = min(Tx_Width_Log2[txSz], 5) + min(Tx_Height_Log2[txSz], 5) - 4
            # eobPt: eob in ( 2^( eobPt - 2 ), 2^( eobPt - 1 ) ]
            eobPt = 1 if eob == 1 else (eob - 1).bit_length() + 1
            ctx = 0 if txClass == TX_CLASS_2D else 1
            name = "EobPt%d" % (16 << eobMultisize)
            cdf = d.cdf[name][ptype][ctx] if eobMultisize <= 4 else d.cdf[name][ptype]
            cw.sym(cdf, eobPt - 1, len(cdf) - 1)
            eobShift = eobPt - 3
            if eobShift >= 0:
                base = (1 << (eobPt - 2)) + 1
                extra = eob - base
                cw.sym(d.cdf["EobExtra"][txSzCtx][ptype][eobPt - 3], (extra >> eobShift) & 1, 2)
                for i in range(1, max(0, eobPt - 2)):
                    sh = max(0, eobPt - 2) - 1 - i
                    cw.lit((extra >> sh) & 1, 1)
            adjTxSz = Adjusted_Tx_Size[txSz]
            bwl = Tx_Width_Log2[adjTxSz]
            width = 1 << bwl
            height = Tx_Height[adjTxSz]
            Quant = [0] * len(quant)
            d.Quant = Quant
            for c in range(eob - 1, -1, -1):
                pos = scan[c]
                level = abs(quant[pos])
                if c == eob - 1:
                    if c == 0:
                        cx = SIG_COEF_CONTEXTS - 4
                    elif c <= (height << bwl) // 8:
                        cx = SIG_COEF_CONTEXTS - 3
                    elif c <= (height << bwl) // 4:
                        cx = SIG_COEF_CONTEXTS - 2
                    else:
                        cx = SIG_COEF_CONTEXTS - 1
                    cx = cx - SIG_COEF_CONTEXTS + SIG_COEF_CONTEXTS_EOB
                    cw.sym(d.cdf["CoeffBaseEob"][txSzCtx][ptype][cx], min(level, 3) - 1, 3)
                else:
                    cx = d.coeff_base_ctx(txSz, txClass, bwl, width, height, pos, Quant)
                    cw.sym(d.cdf["CoeffBase"][txSzCtx][ptype][cx], min(level, 3), 4)
                if level > NUM_BASE_LEVELS:
                    cx = d.coeff_br_ctx(txSz, txClass, pos, Quant)
                    rem = level - 3
                    for idx in range(COEFF_BASE_RANGE // (BR_CDF_SIZE - 1)):
                        k = min(rem, 3)
                        cw.sym(d.cdf["CoeffBr"][min(txSzCtx, TX_32X32)][ptype][cx], k, 4)
                        rem -= k
                        if k < 3:
                            break
                Quant[pos] = min(level, 15)
            for c in range(eob):
                pos = scan[c]
                v = quant[pos]
                if v != 0:
                    sign = 1 if v < 0 else 0
                    if c == 0:
                        cw.sym(d.cdf["DcSign"][ptype][d.dc_sign_ctx(plane, x4, y4, w4, h4)], sign, 2)
                    else:
                        cw.lit(sign, 1)
                    lv = abs(v)
                    if lv > NUM_BASE_LEVELS + COEFF_BASE_RANGE:
                        x_ = lv - COEFF_BASE_RANGE - NUM_BASE_LEVELS
                        length = x_.bit_length()
                        for _ in range(length - 1):
                            cw.lit(0, 1)
                        cw.lit(1, 1)
                        for i in range(length - 2, -1, -1):
                            cw.lit((x_ >> i) & 1, 1)
                    if pos == 0:
                        dcCategory = 1 if sign else 2
                    culLevel += lv
            culLevel = min(63, culLevel)
        if update:
            self.set_ctx(plane, x4, y4, w4, h4, culLevel, dcCategory)
        return eob

    def set_ctx(self, plane, x4, y4, w4, h4, culLevel, dcCategory):
        d = self.d
        for i in range(w4):
            d.AboveLevelContext[plane][x4 + i] = culLevel
            d.AboveDcContext[plane][x4 + i] = dcCategory
        for i in range(h4):
            d.LeftLevelContext[plane][y4 + i] = culLevel
            d.LeftDcContext[plane][y4 + i] = dcCategory

    def recon_txb(self, plane, x, y, txSz, quant, txType):
        d = self.d
        if not any(quant):
            return
        d.Quant = list(quant)
        d.PlaneTxType = DCT_DCT if d.Lossless else (txType if plane == 0 else d.compute_tx_type(plane, txSz, 0, 0))
        d.reconstruct(plane, x, y, txSz)

    def ssd(self, plane, x, y, w, h):
        d = self.d
        src = self.src[plane]
        F = d.CurrFrame[plane]
        ph, pw = len(src), len(src[0])
        t = 0
        for i in range(h):
            if y + i >= ph:
                break
            srow, frow = src[y + i], F[y + i]
            for j in range(min(w, pw - x)):
                e = srow[x + j] - frow[x + j]
                t += e * e
        return t

    def mode_cost(self, plane, mode):
        """Q8 bits of the luma (plane 0) or chroma mode symbols (+ angle delta 0)"""
        d = self.d
        est = EstCoder()
        if plane == 0:
            above = Intra_Mode_Context[d.YModes[d.MiRow - 1][d.MiCol] if d.AvailU else DC_PRED]
            left = Intra_Mode_Context[d.YModes[d.MiRow][d.MiCol - 1] if d.AvailL else DC_PRED]
            est.sym(d.cdf["IntraFrameYMode"][above][left], mode, 13)
        else:
            cdf = d.uv_mode_cdf()
            est.sym(cdf, mode, len(cdf) - 1)
        if d.MiSize >= BLOCK_8X8 and V_PRED <= mode <= D67_PRED:
            est.sym(d.cdf["AngleDelta"][mode - V_PRED], MAX_ANGLE_DELTA, 7)
        return est.bits

    def try_plane(self, plane, mode, txsearch=False):
        """predict + quantize + reconstruct the plane's transform blocks with mode (in decoding
        order: BlockDecoded / LoopfilterTxSizes / coefficient contexts as transform_block sets
        them; the contexts are restored at the end); txsearch: luma transform type by J
        (transform-domain distortion + rate) among TX_SEARCH; -> (SSD, [(x, y, quant, txType)])"""
        d = self.d
        txSz, blocks = self.tx_blocks(plane)
        if plane == 0:
            d.YMode = mode
        else:
            d.UVMode = mode
        saved = ([list(a) for a in d.AboveLevelContext], [list(a) for a in d.AboveDcContext],
                 [list(a) for a in d.LeftLevelContext], [list(a) for a in d.LeftDcContext])
        quants = []
        dist = 0
        for (x, y, bx4, by4) in blocks:
            self.predict(plane, mode, x, y, txSz, bx4, by4)
            res = self.residual_of(plane, x, y, txSz)
            if plane == 0 and txsearch and not d.Lossless and d.get_tx_set(txSz) > 0:
                best = None
                for t in TX_SEARCH:
                    q, dt = self.quant_txb(plane, x, y, txSz, t, res)
                    J = dt + self.lam * self.txb_rate(plane, x, y, txSz, q, t)
                    if best is None or J < best[0]:
                        best = (J, t, q)
                _, txType, q = best
            else:
                txType = DCT_DCT if d.Lossless else self.plane_tx_type(plane, txSz)
                q, _ = self.quant_txb(plane, x, y, txSz, txType, res)
            if not any(q):
                txType = DCT_DCT
            self.recon_txb(plane, x, y, txSz, q, txType)
            quants.append((x, y, q, txType))
            dist += self.ssd(plane, x, y, Tx_Width[txSz], Tx_Height[txSz])
            self.finish_txb(plane, x, y, txSz)
            cul = min(63, sum(abs(v) for v in q))
            dcc = 0 if q[0] == 0 else 1 if q[0] < 0 else 2
            self.set_ctx(plane, x >> 2, y >> 2, Tx_Width[txSz] >> 2, Tx_Height[txSz] >> 2, cul, dcc)
        d.AboveLevelContext, d.AboveDcContext, d.LeftLevelContext, d.LeftDcContext = saved
        return dist, quants

    def finish_txb(self, plane, x, y, txSz):
        """LoopfilterTxSizes / BlockDecoded of a transform block (end of transform_block)"""
        d = self.d
        sx = d.subsampling_x if plane else 0
        sy = d.subsampling_y if plane else 0
        row = (y << sy) >> 2
        col = (x << sx) >> 2
        sbr, sbc = row & 15, col & 15
        for i in range(Tx_Height[txSz] >> 2):
            for j in range(Tx_Width[txSz] >> 2):
                d.LoopfilterTxSizes[plane][(row >> sy) + i][(col >> sx) + j] = txSz
                d.BlockDecoded[plane][(sbr >> sy) + i + 1][(sbc >> sx) + j + 1] = 1

    NCAND = 3

    def best_mode(self, pl, nmodes, s, yMode):
        """intra mode of luma (pl 0) or chroma (pl 1: both planes) with the smallest RD cost;
        single transform block: SATD of every mode first, RD of the NCAND best"""
        d = self.d
        r, c, bSize, _ = s
        planes = (0,) if pl == 0 else (1, 2)
        txSz, blocks = self.tx_blocks(planes[0])
        cands = list(range(nmodes))
        if len(blocks) == 1 and nmodes > self.NCAND:
            x, y, bx4, by4 = blocks[0]
            sc = []
            for m in range(nmodes):
                sat = 0
                for p in planes:
                    if p == 0:
                        d.YMode = m
                    else:
                        d.UVMode = m
                    self.predict(p, m, x, y, txSz, bx4, by4)
                    sat += self.satd(self.residual_of(p, x, y, txSz))
                sc.append((sat * 32768 + self.lam_satd * self.mode_cost(pl, m), m))
            sc.sort()
            cands = [m for _, m in sc[:self.NCAND]]
        best = None
        for m in cands:
            self.restore(s)
            self.set_block(r, c, bSize)
            if yMode is not None:
                d.YMode = yMode
            dist = 0
            est = EstCoder()
            for p in planes:
                dd, q = self.try_plane(p, m)
                dist += dd
                for (x, y, qq, tt) in q:
                    self.code_txb(est, p, x, y, txSz, qq, tt)
            J = dist * 65536 + self.lam * (est.bits + self.mode_cost(pl, m))
            if best is None or J < best[0]:
                best = (J, m)
        return best[1]

    def choose_modes(self, r, c, bSize):
        d = self.d
        s0 = self.snap(r, c, bSize)
        yMode = self.best_mode(0, INTRA_MODES, s0, None)
        uvMode = DC_PRED
        if d.HasChroma:
            self.restore(s0)
            self.set_block(r, c, bSize)
            self.try_plane(0, yMode)
            s1 = self.snap(r, c, bSize)
            uvMode = self.best_mode(1, UV_INTRA_MODES_CFL_NOT_ALLOWED, s1, yMode)
        self.restore(s0)
        self.set_block(r, c, bSize)
        return yMode, uvMode

    def block(self, r, c, bSize, cw, dec=None):
        """search (dec None) or code (dec given) one block with cw; -> (J, decisions)"""
        d = self.d
        self.set_block(r, c, bSize)
        if dec is None:
            dec = self.choose_modes(r, c, bSize)
        yMode, uvMode = dec
        d.YMode = yMode
        d.UVMode = uvMode
        dist, qy = self.try_plane(0, yMode, True)
        qc = []
        if d.HasChroma:
            for p in (1, 2):
                dd, q = self.try_plane(p, uvMode)
                dist += dd
                qc.append(q)
        skip = not any(any(t[2]) for t in qy) and not any(any(t[2]) for qq in qc for t in qq)
        bits0 = cw.bits
        d.skip = int(skip)
        self.mode_info(cw, skip)
        bw4, bh4 = Num_4x4_Blocks_Wide[bSize], Num_4x4_Blocks_High[bSize]
        if skip:
            d.reset_block_context(bw4, bh4)
        for y in range(bh4):
            for x in range(bw4):
                d.YModes[r + y][c + x] = yMode
                if d.HasChroma:
                    d.UVModes[r + y][c + x] = uvMode
                d.InterTxSizes[r + y][c + x] = d.TxSize
        if not skip:
            txSz, _ = self.tx_blocks(0)
            for (x, y, q, t) in qy:
                self.code_txb(cw, 0, x, y, txSz, q, t)
            if d.HasChroma:
                txc, _ = self.tx_blocks(1)
                for k, p in enumerate((1, 2)):
                    for (x, y, q, t) in qc[k]:
                        self.code_txb(cw, p, x, y, txc, q, t)
        for y in range(bh4):
            for x in range(bw4):
                d.Skips[r + y][c + x] = d.skip
                d.TxSizes[r + y][c + x] = d.TxSize
                d.MiSizes[r + y][c + x] = bSize
                d.SegmentIds[r + y][c + x] = 0
                d.PaletteSizes[0][r + y][c + x] = 0
                d.PaletteSizes[1][r + y][c + x] = 0
                d.DeltaLFs[r + y][c + x] = list(d.DeltaLF)
        J = dist * 65536 + self.lam * (cw.bits - bits0)
        return J, dec

    def mode_info(self, cw, skip):
        """intra_frame_mode_info written (no segmentation / CDEF / deltas / palette / filter intra)"""
        d = self.d
        ctx = 0
        if d.AvailU:
            ctx += d.Skips[d.MiRow - 1][d.MiCol]
        if d.AvailL:
            ctx += d.Skips[d.MiRow][d.MiCol - 1]
        cw.sym(d.cdf["Skip"][ctx], 1 if skip else 0, 2)
        above = Intra_Mode_Context[d.YModes[d.MiRow - 1][d.MiCol] if d.AvailU else DC_PRED]
        left = Intra_Mode_Context[d.YModes[d.MiRow][d.MiCol - 1] if d.AvailL else DC_PRED]
        cw.sym(d.cdf["IntraFrameYMode"][above][left], d.YMode, 13)
        if d.MiSize >= BLOCK_8X8 and V_PRED <= d.YMode <= D67_PRED:
            cw.sym(d.cdf["AngleDelta"][d.YMode - V_PRED], MAX_ANGLE_DELTA, 7)
        if d.HasChroma:
            cdf = d.uv_mode_cdf()
            cw.sym(cdf, d.UVMode, len(cdf) - 1)
            if d.MiSize >= BLOCK_8X8 and V_PRED <= d.UVMode <= D67_PRED:
                cw.sym(d.cdf["AngleDelta"][d.UVMode - V_PRED], MAX_ANGLE_DELTA, 7)


# ====================================================================== API

def encode(planes, w, h, chroma, bd, qidx, lf=True, tile_sb=4):
    e = Encoder(planes, w, h, chroma, bd, qidx, lf, tile_sb=tile_sb)
    data = e.encode()
    return data, e


def rgb_to_planes(img, chroma, bd):
    """BT.709 full range (heic_enc.s hw_yuv in Q16) for tests: img = [h][w] of (r, g, b) 8 bit"""
    h, w = len(img), len(img[0])
    sh = bd - 8
    Y = [[0] * w for _ in range(h)]
    U = [[0] * w for _ in range(h)]
    V = [[0] * w for _ in range(h)]
    for y in range(h):
        for x in range(w):
            r, g, b = img[y][x][:3]
            Y[y][x] = ((13933 * r + 46871 * g + 4732 * b) << sh) + 32768 >> 16
            U[y][x] = (((-7509 * r - 25259 * g + 32768 * b) << sh) + (128 << (16 + sh)) + 32768) >> 16
            V[y][x] = (((32768 * r - 29763 * g - 3005 * b) << sh) + (128 << (16 + sh)) + 32768) >> 16
    mx = (1 << bd) - 1
    clip = lambda p: [[min(mx, max(0, v)) for v in row] for row in p]  # noqa: E731
    if chroma == 0:
        return [clip(Y)]
    if chroma == 3:
        return [clip(Y), clip(U), clip(V)]
    cw, ch = (w + 1) >> 1, (h + 1) >> 1
    U2 = [[0] * cw for _ in range(ch)]
    V2 = [[0] * cw for _ in range(ch)]
    for y in range(ch):
        for x in range(cw):
            ys = [min(2 * y, h - 1), min(2 * y + 1, h - 1)]
            xs = [min(2 * x, w - 1), min(2 * x + 1, w - 1)]
            U2[y][x] = (sum(U[a][b] for a in ys for b in xs) + 2) >> 2
            V2[y][x] = (sum(V[a][b] for a in ys for b in xs) + 2) >> 2
    return [clip(Y), clip(U2), clip(V2)]


def main():
    import argparse
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("inp")
    ap.add_argument("out")
    ap.add_argument("--q", type=int, default=60)
    ap.add_argument("--chroma", default="420")
    ap.add_argument("--bits", type=int, default=8)
    ap.add_argument("--lossless", action="store_true")
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    import subprocess
    raw = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", a.inp, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                         capture_output=True, check=True).stdout
    probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height",
                            "-of", "csv=p=0", a.inp], capture_output=True, text=True, check=True).stdout
    w, h = map(int, probe.strip().split(","))
    img = [[raw[(y * w + x) * 3:(y * w + x) * 3 + 3] for x in range(w)] for y in range(h)]
    ch = {"420": 1, "444": 3, "400": 0}[a.chroma]
    planes = rgb_to_planes(img, ch, a.bits)
    qidx = quality_qidx(a.q, a.lossless)
    data, e = encode(planes, w, h, ch, a.bits, qidx)
    Path(a.out).write_bytes(data)
    print("%d bytes, qidx %d" % (len(data), qidx))
    if a.check:
        fr = R.Decoder(True, "recon").decode(data)[0]
        bad = 0
        for p in range(len(fr.planes)):
            for y, row in enumerate(fr.planes[p]):
                if row != e.recon[p][y][:len(row)]:
                    bad += 1
        print("reconstruction %s" % ("identical" if not bad else "DIFFERS (%d rows)" % bad))


if __name__ == "__main__":
    main()
