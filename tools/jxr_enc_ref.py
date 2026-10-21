#!/usr/bin/env python3
"""Reference JPEG XR encoder (ITU-T T.832) -- the model of ASMLIB/src/img/jxr_enc.s.

The assembly encoder produces the same bytes as this file for the same input and options
(tools/jxr_check.py --only enc compares them), so a difference points at the first
macroblock that went wrong (encode(..., log=list) records the bit position after every MB).

Codestream layout (what WIC reads; our decoder jxr_ent.s is the inverse):
  * spatial mode, one tile, no index table, soft tiling, no windowing, short header
  * internal colour format YUV444 / YUV422 / YUV420 (BGR input) or YONLY (planar alpha)
  * lossless: unscaled arithmetic, OVERLAP 0, every QP index 0
  * lossy: scaled arithmetic (samples << 3), OVERLAP 1, QP indices from the quality like
    jxrlib (DPK_QPS tables, U / V / HP separate), DC and LP share the QP
  * container: Bgr24 or Bgra32 (alpha as a second, planar YONLY codestream, always lossless)

Entry points
  encode(bgra, quality=90, lossless=False, chroma=444, alpha=None, log=None) -> bytes
      bgra: numpy uint8 (h, w, 4) straight alpha; alpha None = only when some alpha < 255
  codestream(planes, cf, lossless, qp6, log=None) -> bytes   (planes: list of int arrays)
  qp_indices(quality, chroma) -> (Y, U, V, YHP, UHP, VHP)

stdlib + numpy only.
"""

import struct

import numpy as np

# ------------------------------------------------------------------ tables
YONLY, YUV420, YUV422, YUV444 = 0, 1, 2, 3
MAXTOTAL = 32767
SCAN_LP = [0, 1, 4, 5, 2, 8, 6, 9, 3, 12, 10, 7, 13, 11, 14, 15]
SCAN_H = [0, 1, 4, 5, 2, 8, 6, 9, 3, 12, 10, 7, 13, 11, 14, 15]
SCAN_V = [0, 4, 8, 5, 1, 12, 9, 6, 2, 13, 3, 15, 7, 10, 14, 11]
F2R = [0, 2, 1, 7, 8, 15, 12, 11, 4, 3, 5, 6, 13, 14, 9, 10]      # F index -> raster index
CBP_QUAD = [(0, 0), (0, 1), (1, 0), (1, 1), (0, 2), (0, 3), (1, 2), (1, 3),
            (2, 0), (2, 1), (3, 0), (3, 1), (2, 2), (2, 3), (3, 2), (3, 3)]  # cbp bit -> (row, col)
GOUT0 = [0, 15, 3, 12, 1, 2, 4, 8, 5, 6, 9, 10, 7, 11, 13, 14]
GFLC0 = [0, 2, 1, 2, 2, 0]
GOFF0 = [0, 4, 2, 8, 12, 1]
ALPHA = [5, 4, 8, 7, 7, 12, 6, 6, 12, 6, 6, 7, 7, 12, 6, 6, 12, 6, 6, 7, 7]
UV_POS = {YUV420: [1, 2, 3], YUV422: [4, 1, 2, 3, 5, 6, 7]}         # joint UV LP positions

LENS = {
    9: [[1, 3, 3, 5, 5, 4, 4, 5, 5], [3, 1, 5, 3, 7, 3, 7, 4, 6]],
    8: [[2, 3, 5, 4, 2, 3, 5, 3]],
    4: [[1, 2, 3, 3]],
    5: [[1, 2, 3, 4, 4], [1, 3, 3, 3, 3]],
    6: [[1, 5, 3, 5, 2, 4], [2, 4, 2, 4, 2, 3], [4, 4, 2, 2, 2, 3], [5, 5, 2, 1, 4, 3]],
    7: [[2, 2, 2, 3, 4, 5, 5], [1, 2, 3, 4, 5, 6, 6]],
    12: [[5, 6, 7, 7, 5, 3, 5, 1, 5, 4, 5, 3],
         [4, 5, 6, 6, 4, 3, 5, 2, 3, 3, 5, 3],
         [2, 3, 7, 7, 5, 3, 7, 3, 3, 3, 7, 4],
         [3, 2, 7, 5, 5, 3, 7, 3, 5, 3, 6, 3],
         [3, 1, 7, 4, 7, 3, 8, 4, 7, 4, 8, 5]],
}
CODES = {
    9: [["1", "010", "011", "00000", "00011", "0010", "0011", "00001", "00010"],
        ["011", "1", "00001", "001", "0000000", "010", "0000001", "0001", "000001"]],
    4: [["1", "01", "000", "001"]],
    5: [["1", "01", "001", "0000", "0001"], ["1", "000", "001", "010", "011"]],
    6: [["1", "00000", "001", "00001", "01", "0001"],
        ["01", "0000", "10", "0001", "11", "001"],
        ["0000", "0001", "01", "10", "11", "001"],
        ["00000", "00001", "01", "1", "0001", "001"]],
    8: [["10", "001", "00001", "0001", "11", "010", "00000", "011"]],
    7: [["01", "10", "11", "001", "0001", "00000", "00001"],
        ["1", "01", "001", "0001", "00001", "000000", "000001"]],
    12: [["00001", "000001", "0000000", "0000001", "00100", "010", "00101", "1", "00110", "0001", "00111", "011"],
         ["0010", "00010", "000000", "000001", "0011", "010", "00011", "11", "011", "100", "00001", "101"],
         ["11", "001", "0000000", "0000001", "00001", "010", "0000010", "011", "100", "101", "0000011", "0001"],
         ["001", "11", "0000000", "00001", "00010", "010", "0000001", "011", "00011", "100", "000001", "101"],
         ["010", "1", "0000001", "0001", "0000010", "011", "00000000", "0010", "0000011", "0011", "00000001", "00001"]],
}
MAXT = {4: 1, 5: 2, 6: 4, 7: 2, 8: 1, 9: 2, 12: 5}
SECOND = {4: 0, 5: 0, 6: 1, 7: 0, 8: 0, 9: 0, 12: 1}
THR, MEM = 8, 8

# quality -> QP index tables of jxrlib (Y, U, V, YHP, UHP, VHP)
DPK_QPS_420 = [[66, 65, 70, 72, 72, 77], [59, 58, 63, 64, 63, 68], [52, 51, 57, 56, 56, 61],
               [48, 48, 54, 51, 50, 55], [43, 44, 48, 46, 46, 49], [37, 37, 42, 38, 38, 43],
               [26, 28, 31, 27, 28, 31], [16, 17, 22, 16, 17, 21], [10, 11, 13, 10, 10, 13],
               [5, 5, 6, 5, 5, 6], [2, 2, 3, 2, 2, 2], [2, 2, 3, 2, 2, 2]]
DPK_QPS_8 = [[67, 79, 86, 72, 90, 98], [59, 74, 80, 64, 83, 89], [53, 68, 75, 57, 76, 83],
             [49, 64, 71, 53, 70, 77], [45, 60, 67, 48, 67, 74], [40, 56, 62, 42, 59, 66],
             [33, 49, 55, 35, 51, 58], [27, 44, 49, 28, 45, 50], [20, 36, 42, 20, 38, 44],
             [13, 27, 34, 13, 28, 34], [7, 17, 21, 8, 17, 21], [2, 5, 6, 2, 5, 6]]


def qp_indices(quality, chroma=444):
    """quality 1..100 -> 6 QP indices; integer arithmetic (per mille) so asm matches exactly:
    f = quality * 10 (0.1 % steps); 4:4:4 remaps f > 800 to 800 + (f - 800) * 3 / 2."""
    q = max(1, min(100, int(quality)))
    f = q * 10
    tab = DPK_QPS_420 if chroma in (420, 422) else DPK_QPS_8
    if chroma == 444 and f > 800:
        f = 800 + (f - 800) * 3 // 2
    qi = min(f // 100, 11)
    qf = f - qi * 100 if qi < 11 else 0
    r0 = tab[qi]
    r1 = tab[min(qi + 1, 11)]
    return tuple((r0[j] * (100 - qf) + r1[j] * qf + 50) // 100 for j in range(6))


def qp_value(idx, scaled, shift):
    if idx == 0:
        return 1
    if not scaled:
        if idx < 32:
            man, exp = (idx + 3) >> 2, 0
        elif idx < 48:
            man, exp = (16 + (idx & 15) + 1) >> 1, (idx >> 4) - 2
        else:
            man, exp = 16 + (idx & 15), (idx >> 4) - 3
    elif idx < 16:
        man, exp = idx, shift
    else:
        man, exp = 16 + (idx & 15), (idx >> 4) - 1 + shift
    return man << exp


# ------------------------------------------------------------------ bit writer
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
            self.out.append((self.acc >> self.n) & 255)
        self.acc &= (1 << self.n) - 1

    def code(self, s):
        self.put(int(s, 2), len(s))

    def align(self):
        if self.n:
            self.put(0, 8 - self.n)

    def pos(self):
        return len(self.out) * 8 + self.n

    def data(self):
        self.align()
        return bytes(self.out)


# ------------------------------------------------------------------ adaptive VLC (jxrlib discriminants)
class AH:
    def __init__(self, n):
        self.n = n
        self.initd = False
        self.t = 0
        self.disc = self.disc1 = 0
        self.adapt()

    def adapt(self):
        n = self.n
        if not self.initd:
            self.initd = True
            self.disc = self.disc1 = 0
            self.t = SECOND[n]
            self.lo, self.hi = -(1 << 31), 1 << 30
        dl = dh = self.disc
        if SECOND[n]:
            dh = self.disc1
        change = False
        if dl < self.lo:
            self.t -= 1
            change = True
        elif dh > self.hi:
            self.t += 1
            change = True
        if change:
            self.disc = self.disc1 = 0
        lim = THR * MEM
        self.disc = max(-lim, min(lim, self.disc))
        self.disc1 = max(-lim, min(lim, self.disc1))
        t, mt = self.t, MAXT[n]
        self.lo = -(1 << 31) if t == 0 else -THR
        self.hi = (1 << 30) if t == mt - 1 else THR
        self.delta = self.delta1 = None
        if mt > 1:
            if SECOND[n]:
                self.delta = self._d(t - 1 + (t == 0))
                self.delta1 = self._d(t - (t + 1 == mt))
            else:
                self.delta = self._d(0)

    def _d(self, t):
        return [a - b for a, b in zip(LENS[self.n][t], LENS[self.n][t + 1])]

    def put(self, bw, s, update=True):
        bw.code(CODES[self.n][self.t][s])
        if update and self.delta is not None:
            self.disc += self.delta[s]
            if self.delta1 is not None:
                self.disc1 += self.delta1[s]


class Scan:
    def __init__(self, order):
        self.order = list(order)
        self.total = [0] * 16
        self.reset()

    def reset(self):
        self.total[0] = MAXTOTAL
        for k in range(1, 16):
            self.total[k] = 34 - 2 * k

    def update(self, k):
        self.total[k] += 1
        if k > 1 and self.total[k] > self.total[k - 1]:
            self.order[k], self.order[k - 1] = self.order[k - 1], self.order[k]
            self.total[k], self.total[k - 1] = self.total[k - 1], self.total[k]


class Model:
    def __init__(self, band):
        self.band = band
        self.state = [0, 0]
        self.bits = [8, 8] if band == 0 else ([4, 4] if band == 1 else [0, 0])


W0 = [240, 12, 1]
W1 = [[0, 240, 120, 80, 60, 48, 40, 34, 30, 27, 24, 22, 20, 18, 17, 16],
      [0, 12, 6, 4, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1],
      [0, 16, 8, 5, 4, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1]]
W2 = [120, 37, 2, 120, 18, 1]


def update_model(cf, nch, lm, m):
    lm = list(lm)
    lm[0] *= W0[m.band]
    if cf == YUV420:
        lm[1] *= W2[m.band]
    elif cf == YUV422:
        lm[1] *= W2[3 + m.band]
    else:
        lm[1] *= W1[m.band][nch - 1]
        if m.band == 2:
            lm[1] >>= 4
    for j in range(2):
        ms = m.state[j]
        d = (lm[j] - 70) >> 2
        if d <= -8:
            d = max(d + 4, -16)
            ms += d
            if ms < -8:
                if m.bits[j] == 0:
                    ms = -8
                else:
                    ms = 0
                    m.bits[j] -= 1
        elif d >= 8:
            d = min(d - 4, 15)
            ms += d
            if ms > 8:
                if m.bits[j] >= 15:
                    m.bits[j] = 15
                    ms = 8
                else:
                    ms = 0
                    m.bits[j] += 1
        m.state[j] = ms
        if cf == YONLY or nch == 1:
            break


# ------------------------------------------------------------------ transforms (forward)
def fwd_t2x2h(v, a, b, c, d, r):
    a2, b2, c2, d2 = v[a], v[b], v[c], v[d]
    a1 = a2 + d2
    b1 = b2 - c2
    t = (a1 - b1 + r) >> 1
    dd = t - c2
    cc = t - d2
    v[a], v[b], v[c], v[d] = a1 - dd, b1 + cc, cc, dd


def fwd_odd(v, a, b, c, d):
    A, B, C, D = v[a], v[b], v[c], v[d]
    A += D
    B -= C
    D = ((A + 1) >> 1) - D
    C += (B + 1) >> 1
    D -= (C * 3 + 4) >> 3
    C += (D * 3 + 4) >> 3
    B -= (A * 3 + 4) >> 3
    A += (B * 3 + 4) >> 3
    C -= (A + 1) >> 1
    D += B >> 1
    A += C
    B -= D
    v[a], v[b], v[c], v[d] = A, B, C, D


def fwd_oddodd(v, a, b, c, d):
    A, B, C, D = v[a], -v[b], -v[c], v[d]
    D += A
    C -= B
    t1 = D >> 1
    t2 = C >> 1
    A -= t1
    B += t2
    A += (B * 3 + 4) >> 3
    B -= (A * 3 + 3) >> 2
    A += (B * 3 + 3) >> 3
    B -= t2
    A += t1
    C += B
    D -= A
    v[a], v[b], v[c], v[d] = A, B, C, D


def fct4x4(v):
    """forward PCT on 16 values in raster order (in place); exact inverse of the decoder ICT"""
    fwd_t2x2h(v, 4, 7, 8, 11, 0)
    fwd_t2x2h(v, 1, 2, 13, 14, 0)
    fwd_t2x2h(v, 5, 6, 9, 10, 0)
    fwd_t2x2h(v, 0, 3, 12, 15, 0)
    fwd_oddodd(v, 10, 11, 14, 15)
    fwd_odd(v, 8, 12, 9, 13)
    fwd_odd(v, 2, 3, 6, 7)
    fwd_t2x2h(v, 0, 1, 4, 5, 1)


def t2x2h(v, a, b, c, d, r):
    A, B, C, D = v[a], v[b], v[c], v[d]
    A += D
    B -= C
    t = (A - B + r) >> 1
    c2 = t - D
    d2 = t - C
    v[a], v[b], v[c], v[d] = A - d2, B + c2, c2, d2


def t2x2h_enc(v, a, b, c, d):
    A, B, C, D = v[a], v[b], v[c], v[d]
    A += D
    B -= C
    t1, t2 = D, C
    C = ((A - B) >> 1) - t1
    D = t2 + (B >> 1)
    B += C
    A -= (D * 3 + 4) >> 3
    v[a], v[b], v[c], v[d] = A, B, C, D


def fwd_rotate(v, a, b):
    v[b] -= (v[a] + 1) >> 1
    v[a] += (v[b] + 1) >> 1


def fwd_scale(v, a, b):
    A, B = v[a], v[b]
    B -= (A * 3) >> 4
    B -= A >> 7
    B += A >> 10
    A -= (B * 3) >> 3
    B = (A >> 1) - B
    A -= B
    v[a], v[b] = A, B


def fwd_toddodd(v, a, b, c, d):
    A, B, C, D = v[a], v[b], v[c], v[d]
    D += A
    C -= B
    t1 = D >> 1
    t2 = C >> 1
    A -= t1
    B += t2
    A += (B * 3 + 4) >> 3
    B -= (A * 3 + 2) >> 2
    A += (B * 3 + 6) >> 3
    B -= t2
    A += t1
    C += B
    D -= A
    v[a], v[b], v[c], v[d] = A, B, C, D


def pre4x4(v):
    """OverlapPreFilter4x4 (T.832 Table D.18) on 16 samples (raster of the straddling area)"""
    t2x2h_enc(v, 0, 3, 12, 15)
    t2x2h_enc(v, 1, 2, 13, 14)
    t2x2h_enc(v, 4, 7, 8, 11)
    t2x2h_enc(v, 5, 6, 9, 10)
    fwd_scale(v, 0, 15)
    fwd_scale(v, 1, 14)
    fwd_scale(v, 4, 11)
    fwd_scale(v, 5, 10)
    fwd_rotate(v, 13, 12)
    fwd_rotate(v, 9, 8)
    fwd_rotate(v, 7, 3)
    fwd_rotate(v, 6, 2)
    fwd_toddodd(v, 10, 11, 14, 15)
    t2x2h(v, 0, 12, 3, 15, 0)
    t2x2h(v, 1, 2, 13, 14, 0)
    t2x2h(v, 4, 7, 8, 11, 0)
    t2x2h(v, 5, 6, 9, 10, 0)


def pre4(v):
    """OverlapPreFilter4 (Table D.19) on 4 values"""
    v[0] += v[3]
    v[1] += v[2]
    v[3] -= (v[0] + 1) >> 1
    v[2] -= (v[1] + 1) >> 1
    fwd_rotate(v, 2, 3)
    v[3] = -v[3]
    v[2] = -v[2]
    v[0] -= v[3]
    v[1] -= v[2]
    v[3] += v[0] >> 1
    v[2] += v[1] >> 1
    v[0] -= (v[3] * 3 + 4) >> 3
    v[1] -= (v[2] * 3 + 4) >> 3
    fwd_scale(v, 0, 3)
    fwd_scale(v, 1, 2)
    v[3] += (v[0] + 1) >> 1
    v[2] += (v[1] + 1) >> 1
    v[0] -= v[3]
    v[1] -= v[2]


def pre_filter(img):
    """first-level overlap pre-filter over a whole plane (list of rows, H and W multiples of 4)"""
    H, W = len(img), len(img[0])
    for y in range(4, H, 4):
        for x in range(4, W, 4):
            v = [img[y - 2 + i][x - 2 + j] for i in range(4) for j in range(4)]
            pre4x4(v)
            for i in range(4):
                for j in range(4):
                    img[y - 2 + i][x - 2 + j] = v[i * 4 + j]
    for x in range(4, W, 4):
        for r in (0, 1, H - 2, H - 1):
            v = img[r][x - 2:x + 2]
            pre4(v)
            img[r][x - 2:x + 2] = v
    for y in range(4, H, 4):
        for c in (0, 1, W - 2, W - 1):
            v = [img[y - 2 + i][c] for i in range(4)]
            pre4(v)
            for i in range(4):
                img[y - 2 + i][c] = v[i]
    for r, c in ((0, 0), (0, W - 2), (H - 2, 0), (H - 2, W - 2)):
        v = [img[r][c], img[r][c + 1], img[r + 1][c], img[r + 1][c + 1]]
        pre4(v)
        img[r][c], img[r][c + 1], img[r + 1][c], img[r + 1][c + 1] = v


def to_f(r):
    return [r[F2R[k]] for k in range(16)]


# ------------------------------------------------------------------ colour
def fwd_color(r, g, b):
    """FwdColorFmtConvert1 on int arrays -> Y, U, V"""
    v = b - r
    t = r - g + ((v + 1) >> 1)
    y = g + (t >> 1)
    return y, -t, v


# ------------------------------------------------------------------ CBP helpers
def cbp_pred_enc(cbp, st, left_cbp, top_cbp, at_left, at_top):
    """inverse of the decoder's CBP prediction (predCBPCDec), 16-bit luma / 444 layout"""
    if st == 0:
        x = cbp
        x ^= (x & 0x3300) << 2
        x ^= (x & 0xcc) << 6
        x ^= (x & 0x33) << 2
        x ^= 0x20 & (x << 1)
        x ^= 0x10 & (x << 3)
        x ^= 0x02 & (x << 1)
        if at_left:
            p = 1 if at_top else (top_cbp >> 10) & 1
        else:
            p = (left_cbp >> 5) & 1
        return x ^ p
    if st == 2:
        return cbp ^ 0xffff
    return cbp


def cbp_model(X, c1, cbp, nb=16):
    n = bin(cbp).count("1")
    mu = 16 // nb
    k = 3
    X.cm_c0[c1] = max(-16, min(15, X.cm_c0[c1] + n * mu - k))
    X.cm_c1[c1] = max(-16, min(15, X.cm_c1[c1] + (nb - n) * mu - k))
    if X.cm_c0[c1] < 0:
        X.cm_st[c1] = 1 if X.cm_c0[c1] < X.cm_c1[c1] else 2
    elif X.cm_c1[c1] < 0:
        X.cm_st[c1] = 2
    else:
        X.cm_st[c1] = 0


def cbp_pred_enc_c(cbp, st, nb, left_cbp, top_cbp, at_left, at_top):
    """inverse of the 420 (4 bits, 2x2) / 422 (8 bits, 2 wide x 4 tall) chroma CBP prediction"""
    if st == 0:
        tb = 2 if nb == 4 else 6
        x = cbp
        for r in range(nb // 2 - 1, 0, -1):
            x ^= (3 << (2 * r)) & (x << 2)
        x ^= 0x02 & (x << 1)
        if at_left:
            p = 1 if at_top else (top_cbp >> tb) & 1
        else:
            p = (left_cbp >> 1) & 1
        return x ^ p
    if st == 2:
        return cbp ^ ((1 << nb) - 1)
    return cbp


def quad_class(pat):
    for c in range(6):
        for b in range(1 << GFLC0[c]):
            if GOUT0[GOFF0[c] + b] == pat:
                return c, b
    raise ValueError(pat)


def put_mask4(X, bw, ah, m, update=True):
    """count (VLC) + position bits of a 4-bit mask (quadrant masks, chroma patterns)"""
    n = bin(m).count("1")
    if ah is not None:
        ah.put(bw, n if ah.n == 5 else n - 1, update)
    if n == 1:
        bw.put(m.bit_length() - 1, 2)
    elif n == 2:
        if m == 3:
            bw.put(0, 2)
        elif m == 5:
            bw.put(1, 2)
        else:
            k = [6, 9, 10, 12].index(m)
            bw.put(2 + (k >> 1), 2)
            bw.put(k & 1, 1)
    elif n == 3:
        bw.put((0xF ^ m).bit_length() - 1, 2)


# ------------------------------------------------------------------ entropy context
class Ctx:
    def __init__(self):
        self.ah = [AH(n) for n in ALPHA]
        self.cbpcy1 = AH(5)
        self.cbpq = AH(5)
        self.cbpq9 = AH(9)
        self.scan_lp, self.scan_h, self.scan_v = Scan(SCAN_LP), Scan(SCAN_H), Scan(SCAN_V)
        self.mdc, self.mlp, self.mac = Model(0), Model(1), Model(2)
        self.cm_c0 = [-4, -4]
        self.cm_c1 = [4, 4]
        self.cm_st = [0, 0]
        self.lp_zero = 1
        self.lp_max = 1

    def adapt(self):
        for a in self.ah:
            a.adapt()
        self.cbpcy1.adapt()
        self.cbpq.adapt()
        self.cbpq9.adapt()


def put_run(X, bw, run, maxrun):
    if maxrun < 5:
        for r in range(1, 5):
            if r == run:
                if maxrun != r:
                    bw.put(1, 1)
                return
            bw.put(0, 1)
        return
    RUNBIN = [-1, -1, -1, -1, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0]
    RUNFLC = [0, 0, 1, 1, 3, 0, 0, 1, 1, 2, 0, 0, 0, 0, 1]
    RUNREMAP = [1, 2, 3, 5, 7, 1, 2, 3, 5, 7, 1, 2, 3, 4, 5]
    b = RUNBIN[maxrun]
    for idx in range(5):
        base = RUNREMAP[idx + b * 5]
        f = RUNFLC[idx + b * 5]
        if base <= run < base + (1 << f):
            X.ah[0].put(bw, idx, update=False)
            bw.put(run - base, f)
            return
    raise ValueError("run")


def put_level(bw, lev, ah):
    """absolute level >= 2"""
    remap = [2, 3, 4, 6, 10, 14]
    flen = [0, 0, 1, 2, 2, 2]
    for idx in range(6):
        if remap[idx] <= lev < remap[idx] + (1 << flen[idx]):
            ah.put(bw, idx)
            bw.put(lev - remap[idx], flen[idx])
            return
    x = lev - 2
    fixed = x.bit_length() - 1
    ah.put(bw, 6)
    if fixed < 19:
        bw.put(fixed - 4, 4)
    elif fixed < 22:
        bw.put(15, 4)
        bw.put(fixed - 19, 2)
    else:
        bw.put(15, 4)
        bw.put(3, 2)
        bw.put(fixed - 22, 3)
    bw.put(x - (1 << fixed), fixed)


def put_block(X, bw, levels, start, off, chroma, loc0=None):
    """run-level coding of levels[start..15] (scan positions) -> number of nonzero levels.
    loc0: initial location (16 - n for the joint UV LP block), default = start"""
    ah = X.ah
    a1 = off + chroma * 3
    nz = [(k, levels[k]) for k in range(start, len(levels)) if levels[k] != 0]
    runs = []
    prev = start
    for k, v in nz:
        runs.append(k - prev)
        prev = k + 1
    n = len(nz)
    loc = start if loc0 is None else loc0
    v0 = nz[0][1]
    sr = 1 if runs[0] == 0 else 0
    srn = 0 if n == 1 else (1 if runs[1] == 0 else 2)
    ah[a1].put(bw, sr | (2 if abs(v0) > 1 else 0) | (srn << 2))
    bw.put(1 if v0 < 0 else 0, 1)
    cont = sr & srn
    if abs(v0) > 1:
        put_level(bw, abs(v0), ah[6 + off + cont])
    if sr == 0:
        put_run(X, bw, runs[0], 15 - loc)
    loc += runs[0] + 1
    for i in range(1, n):
        v = nz[i][1]
        sr = srn & 1
        if sr == 0:
            put_run(X, bw, runs[i], 15 - loc)
        loc += runs[i] + 1
        srn = 0 if i + 1 == n else (1 if runs[i + 1] == 0 else 2)
        idx = (1 if abs(v) > 1 else 0) | (srn << 1)
        if loc < 15:
            ah[a1 + 1 + cont].put(bw, idx)
        elif loc == 15:
            bw.code({0: "0", 2: "10", 1: "110", 3: "111"}[idx])
        else:
            bw.put(idx, 1)
        bw.put(1 if v < 0 else 0, 1)
        cont &= srn
        if abs(v) > 1:
            put_level(bw, abs(v), ah[6 + off + cont])
    return n


def put_refine(bw, vals, mbits):
    if mbits == 0:
        return
    m = (1 << mbits) - 1
    for v in vals:
        a = abs(v)
        bw.put(a & m, mbits)
        if (a >> mbits) == 0 and (a & m):
            bw.put(1 if v < 0 else 0, 1)


def hi(v, mbits):
    a = abs(v) >> mbits
    return -a if v < 0 else a


def put_cbplp(X, bw, cbp, n):
    if n == 1:
        bw.put(cbp, 1)
        return
    mx = (1 << n) - 1
    z, m = X.lp_zero, X.lp_max
    if not (z <= 0 or m < 0):
        bw.put(cbp, n)
    else:
        prob = mx if m < z else 0
        if cbp == prob:
            bw.put(0, 1)
        else:
            bw.put(1, 1)
            v = cbp ^ prob
            if v == 1:
                bw.put(0, n - 1)
            else:
                bw.put(v >> 1, n - 1)
                bw.put(v & 1, 1)
    X.lp_zero = max(-8, z - 3) if cbp == 0 else min(7, z + 1)
    X.lp_max = max(-8, m - 3) if cbp == mx else min(7, m + 1)


# ------------------------------------------------------------------ the encoder
def quant(v, q, dead):
    """dead = 0: round to nearest; 1: HP dead zone (offset 3q/8)"""
    if q == 1:
        return v
    off = (q * 3 + 1) >> 3 if dead else q >> 1
    a = (abs(v) + off) // q
    return -a if v < 0 else a


class PlaneEnc:
    """one codestream plane: channels (lists of rows, extended to MB multiples, samples already
    shifted / colour converted), cf, quantizers"""

    def __init__(self, chans, cf, scaled, overlap, qpi):
        self.cf = cf
        self.nch = len(chans)
        self.scaled = scaled
        self.overlap = overlap
        self.qpi = qpi                      # (dc/lp per channel), (hp per channel)
        H, W = len(chans[0]), len(chans[0][0])
        self.mbw, self.mbh = W // 16, H // 16
        self.chans = chans

    def channel_geom(self, c):
        if c == 0 or self.cf not in (YUV420, YUV422):
            return 4, 4
        return (2, 2) if self.cf == YUV420 else (2, 4)

    def transform(self):
        """first level (pre-filter + FCT per block), DC grids, second level, quantization ->
        self.lp[c][my][mx] (16 F coefficients, quantized), self.hp[c][my][mx][(r, cc)]"""
        cf, nch = self.cf, self.nch
        self.lp = []
        self.hp = []
        for c in range(nch):
            img = [list(map(int, row)) for row in self.chans[c]]
            if self.overlap >= 1:
                pre_filter(img)
            bw, bh = self.channel_geom(c)
            qdc = self.qdc[c]
            qlp = self.qlp[c]
            qhp = self.qhp[c]
            lpc = [[None] * self.mbw for _ in range(self.mbh)]
            hpc = [[None] * self.mbw for _ in range(self.mbh)]
            for my in range(self.mbh):
                for mx in range(self.mbw):
                    dcs = []
                    blocks = {}
                    for r in range(bh):
                        for cc in range(bw):
                            y0 = (my * bh + r) * 4
                            x0 = (mx * bw + cc) * 4
                            v = [img[y0 + i][x0 + j] for i in range(4) for j in range(4)]
                            fct4x4(v)
                            dcs.append(v[0])
                            f = to_f(v)
                            blocks[(r, cc)] = [0] + [quant(f[k], qhp, 1) for k in range(1, 16)]
                    if self.scaled and c > 0:
                        dcs = [(d + 1) >> 1 for d in dcs]        # decoder doubles chroma DCs
                    L = self.lp_forward(dcs, c)
                    lpc[my][mx] = [quant(L[0], qdc, 0)] + [quant(L[k], qlp, 0) for k in range(1, 16)]
                    hpc[my][mx] = blocks
            self.lp.append(lpc)
            self.hp.append(hpc)

    def lp_forward(self, dcs, c):
        """DC values of the blocks of one MB channel -> 16 LP coefficients (F layout)"""
        bw, bh = self.channel_geom(c)
        if (bw, bh) == (4, 4):
            v = list(dcs)
            fct4x4(v)
            return to_f(v)
        L = [0] * 16
        if self.cf == YUV420:
            v = list(dcs)                     # [[a, b], [c, d]] raster
            # inverse of the decoder's t2x2h(v, 0, 1, 2, 3, 0)
            fwd_t2x2h(v, 0, 1, 2, 3, 0)
            L[0], L[1], L[2], L[3] = v
            return L
        # 4:2:2: two 2x2 halves (top, bottom), then a 1-D step on their DCs
        top = list(dcs[0:4])
        bot = list(dcs[4:8])
        fwd_t2x2h(top, 0, 1, 2, 3, 0)
        fwd_t2x2h(bot, 0, 1, 2, 3, 0)
        # decoder: x1 = a - ((b + 1) >> 1) (top), x0 = b + x1 (bottom); a = L0, b = L4
        t, b0 = top[0], bot[0]
        bb = b0 - t
        a = t + ((bb + 1) >> 1)
        L[0], L[4] = a, bb
        L[1], L[2], L[3] = top[1], top[2], top[3]
        L[5], L[6], L[7] = bot[1], bot[2], bot[3]
        return L

    # -------------------------------------------------------------- entropy coding
    def encode(self, bw, log=None):
        cf, nch = self.cf, self.nch
        X = Ctx()
        dcq = {}
        cbps = {}
        for my in range(self.mbh):
            for mx in range(self.mbw):
                if mx % 16 == 0:
                    X.scan_lp.reset()
                    X.scan_h.reset()
                    X.scan_v.reset()
                if (my, mx) != (0, 0) and (mx % 16 == 1 or mx == 0):
                    X.adapt()
                self.encode_mb(X, bw, mx, my, dcq, cbps)
                if log is not None:
                    log.append(bw.pos())

    def encode_mb(self, X, bw, mx, my, dcq, cbps):
        cf, nch = self.cf, self.nch
        Q = [self.lp[c][my][mx] for c in range(nch)]
        # ---------------- DC / LP prediction
        left, top = mx > 0, my > 0
        if not left and not top:
            dcmode = 3
        elif not left:
            dcmode = 1
        elif not top:
            dcmode = 0
        else:
            iL = dcq[(0, my, mx - 1)][0]
            iT = dcq[(0, my - 1, mx)][0]
            iTL = dcq[(0, my - 1, mx - 1)][0]
            sh = abs(iTL - iL)
            sv = abs(iTL - iT)
            if cf != YONLY:
                scale = 8 if cf == YUV420 else (4 if cf == YUV422 else 2)
                sh *= scale
                sv *= scale
                for c in (1, 2):
                    tl = dcq[(c, my - 1, mx - 1)][0]
                    sh += abs(tl - dcq[(c, my, mx - 1)][0])
                    sv += abs(tl - dcq[(c, my - 1, mx)][0])
            dcmode = 1 if sh * 4 < sv else (0 if sv * 4 < sh else 2)
        res = []
        for c in range(nch):
            L = Q[c]
            R = list(L)
            if dcmode == 1:
                R[0] -= dcq[(c, my - 1, mx)][0]
            elif dcmode == 0:
                R[0] -= dcq[(c, my, mx - 1)][0]
            elif dcmode == 2:
                rr = 1 if (c > 0 and cf in (YUV420, YUV422)) else 0
                R[0] -= (dcq[(c, my, mx - 1)][0] + dcq[(c, my - 1, mx)][0] + rr) >> 1
            if c > 0 and cf == YUV420:
                kl, kt = (1,), (2,)
            elif c > 0 and cf == YUV422:
                kl, kt = (1, 4, 5), None
            else:
                kl, kt = (1, 2, 3), (4, 8, 12)
            if dcmode == 1:
                T = dcq[(c, my - 1, mx)]
                if kt is None:
                    # 422 top: L2 += T6, L6 += L2 (own, after), L4 += T4
                    R[2] -= T[6]
                    R[6] -= L[2]
                    R[4] -= T[4]
                else:
                    for k in kt:
                        R[k] -= T[k]
            elif dcmode == 0:
                Lf = dcq[(c, my, mx - 1)]
                for k in kl:
                    R[k] -= Lf[k]
            dcq[(c, my, mx)] = list(L)
            res.append(R)
        # ---------------- DC
        mb_ = X.mdc.bits
        lm = [0, 0]
        if cf == YONLY:
            for c in range(nch):
                dc = res[c][0]
                b = mb_[c > 0]
                q = abs(dc) >> b
                bw.put(1 if q else 0, 1)
                if q:
                    put_level(bw, q + 1, X.ah[3 if c == 0 else 4])
                    lm[c > 0] += 1
                bw.put(abs(dc) & ((1 << b) - 1), b)
                if dc:
                    bw.put(1 if dc < 0 else 0, 1)
        else:
            qs = [abs(res[c][0]) >> mb_[c > 0] for c in range(3)]
            X.ah[2].put(bw, (4 if qs[0] else 0) | (2 if qs[1] else 0) | (1 if qs[2] else 0))
            for c in range(3):
                dc = res[c][0]
                b = mb_[c > 0]
                if qs[c]:
                    put_level(bw, qs[c] + 1, X.ah[3 if c == 0 else 4])
                    lm[c > 0] += 1
                bw.put(abs(dc) & ((1 << b) - 1), b)
                if dc:
                    bw.put(1 if dc < 0 else 0, 1)
        update_model(cf, nch, lm, X.mdc)
        # ---------------- LP
        mb_ = X.mlp.bits
        lm = [0, 0]
        if cf in (YUV420, YUV422):
            self.encode_lp_uv(X, bw, res, mb_, lm)
        else:
            order = X.scan_lp.order
            cbp = 0
            for c in range(nch):
                if any(abs(res[c][k]) >> mb_[c > 0] for k in range(1, 16)):
                    cbp |= 1 << c
            put_cbplp(X, bw, cbp, nch)
            for c in range(nch):
                b = mb_[c > 0]
                if cbp & (1 << c):
                    order = X.scan_lp.order
                    lv = [0] + [hi(res[c][order[k]], b) for k in range(1, 16)]
                    lm[c > 0] += put_block(X, bw, lv, 1, 5, 1 if c > 0 else 0)
                    for k in range(1, 16):
                        if lv[k]:
                            X.scan_lp.update(k)
                put_refine(bw, res[c][1:16], b)
        update_model(cf, nch, lm, X.mlp)
        # ---------------- HP
        L = Q[0]
        sh = abs(L[1]) + abs(L[2]) + abs(L[3])
        sv = abs(L[4]) + abs(L[8]) + abs(L[12])
        if cf != YONLY:
            ih, iv = (1, 4) if cf == YUV444 else (1, 2)
            for c in (1, 2):
                sh += abs(Q[c][ih])
                sv += abs(Q[c][iv])
                if cf == YUV422:
                    sh += abs(Q[c][5])
                    sv += abs(Q[c][6])
        acmode = 1 if sh * 4 < sv else (0 if sv * 4 < sh else 2)
        scan = X.scan_v if acmode == 1 else X.scan_h
        mb_ = X.mac.bits
        hres = []
        for c in range(nch):
            B = self.hp[c][my][mx]
            d = {}
            for (r, cc), v in B.items():
                h = list(v)
                if acmode == 0 and cc > 0:
                    for k in (1, 2, 3):
                        h[k] -= B[(r, cc - 1)][k]
                if acmode == 1 and r > 0:
                    for k in (4, 8, 12):
                        h[k] -= B[(r - 1, cc)][k]
                d[(r, cc)] = h
            hres.append(d)
        border = [self.block_order(c) for c in range(nch)]
        cbpc = []
        for c in range(nch):
            cb = 0
            for bit, rc in enumerate(border[c]):
                if any(abs(hres[c][rc][k]) >> mb_[c > 0] for k in range(1, 16)):
                    cb |= 1 << bit
            cbpc.append(cb)
        self.encode_cbp(X, bw, cbpc, cbps, mx, my)
        lm = [0, 0]
        for c in range(nch):
            b = mb_[c > 0]
            for bit, rc in enumerate(border[c]):
                h = hres[c][rc]
                if cbpc[c] & (1 << bit):
                    order = scan.order
                    lv = [0] + [hi(h[order[k]], b) for k in range(1, 16)]
                    lm[c > 0] += put_block(X, bw, lv, 1, 13, 1 if c > 0 else 0)
                    for k in range(1, 16):
                        if lv[k]:
                            scan.update(k)
                put_refine(bw, h[1:16], b)
        update_model(cf, nch, lm, X.mac)

    def block_order(self, c):
        if c == 0 or self.cf not in (YUV420, YUV422):
            return CBP_QUAD
        if self.cf == YUV420:
            return [(0, 0), (0, 1), (1, 0), (1, 1)]
        return [(r, q) for r in range(4) for q in range(2)]

    def encode_lp_uv(self, X, bw, res, mb_, lm):
        cf = self.cf
        order = X.scan_lp.order
        b0 = mb_[0]
        pos = UV_POS[cf]
        nuv = 2 * len(pos)
        uv = [hi(res[1 + (i & 1)][pos[i >> 1]], mb_[1]) for i in range(nuv)]
        cbp = (1 if any(abs(res[0][k]) >> b0 for k in range(1, 16)) else 0) | (2 if any(uv) else 0)
        put_cbplp(X, bw, cbp, 2)
        if cbp & 1:
            lv = [0] + [hi(res[0][order[k]], b0) for k in range(1, 16)]
            lm[0] += put_block(X, bw, lv, 1, 5, 0)
            for k in range(1, 16):
                if lv[k]:
                    X.scan_lp.update(k)
        put_refine(bw, res[0][1:16], b0)
        if cbp & 2:
            lm[1] += put_block(X, bw, uv, 0, 5, 1, loc0=16 - nuv)
        for k in range(1, len(pos) + 1):
            for c in (1, 2):
                put_refine(bw, [res[c][k]], mb_[1])

    def encode_cbp(self, X, bw, cbpc, cbps, mx, my):
        cf, nch = self.cf, self.nch
        diffs = []
        if cf == YONLY:
            for c in range(nch):
                lc = cbps.get((c, my, mx - 1), 0)
                tc = cbps.get((c, my - 1, mx), 0)
                d = cbp_pred_enc(cbpc[c], X.cm_st[0 if c == 0 else 1], lc, tc, mx == 0, my == 0)
                cbp_model(X, 0 if c == 0 else 1, cbpc[c])
                cbps[(c, my, mx)] = cbpc[c]
                qmask = sum(1 << q for q in range(4) if (d >> (4 * q)) & 15)
                put_mask4(X, bw, X.cbpcy1, qmask)
                for q in range(4):
                    if qmask & (1 << q):
                        cl, bits = quad_class((d >> (4 * q)) & 15)
                        X.cbpq.put(bw, cl - 1)
                        bw.put(bits, GFLC0[cl])
            return
        for c in range(3):
            lc = cbps.get((c, my, mx - 1), 0)
            tc = cbps.get((c, my - 1, mx), 0)
            c1 = 0 if c == 0 else 1
            if c == 0 or cf == YUV444:
                diffs.append(cbp_pred_enc(cbpc[c], X.cm_st[c1], lc, tc, mx == 0, my == 0))
                cbp_model(X, c1, cbpc[c])
            else:
                nb = 4 if cf == YUV420 else 8
                diffs.append(cbp_pred_enc_c(cbpc[c], X.cm_st[1], nb, lc, tc, mx == 0, my == 0))
                cbp_model(X, 1, cbpc[c], nb)
            cbps[(c, my, mx)] = cbpc[c]
        dy, du, dv = diffs
        # chroma bits per quadrant
        if cf == YUV444:
            qu = [(du >> (4 * q)) & 15 for q in range(4)]
            qv = [(dv >> (4 * q)) & 15 for q in range(4)]
        elif cf == YUV420:
            qu = [(du >> q) & 1 for q in range(4)]
            qv = [(dv >> q) & 1 for q in range(4)]
        else:
            # 4:2:2 chroma blocks 2 wide x 4 tall: quadrant (qy, qx) holds bits 4qy+qx, 4qy+2+qx
            qu, qv = [], []
            for q in range(4):
                qy, qx = q >> 1, q & 1
                qu.append(((du >> (4 * qy + qx)) & 1) | (((du >> (4 * qy + 2 + qx)) & 1) << 1))
                qv.append(((dv >> (4 * qy + qx)) & 1) | (((dv >> (4 * qy + 2 + qx)) & 1) << 1))
        qmask = 0
        for q in range(4):
            if ((dy >> (4 * q)) & 15) or qu[q] or qv[q]:
                qmask |= 1 << q
        put_mask4(X, bw, X.cbpcy1, qmask)
        for q in range(4):
            if not qmask & (1 << q):
                continue
            py = (dy >> (4 * q)) & 15
            cy, by = quad_class(py)
            pu, pv = qu[q], qv[q]
            chroma = bool(pu or pv)
            if not chroma:
                sym = {1: 1, 2: 3, 3: 5, 4: 7, 5: 8}[cy]
            else:
                sym = {0: 0, 1: 2, 2: 4, 3: 6, 4: 6, 5: 6}[cy]
            X.cbpq9.put(bw, sym)
            if chroma:
                bw.code("1" if (pu and not pv) else ("01" if (pv and not pu) else "00"))
                if cy >= 3:
                    bw.code({3: "1", 4: "01", 5: "00"}[cy])
            bw.put(by, GFLC0[cy])
            if cf == YUV444:
                for pc in (pu, pv):
                    if pc:
                        put_mask4(X, bw, X.ah[1], pc, update=False)
            elif cf == YUV422:
                for pc in (pu, pv):
                    if pc:
                        bw.code({1: "1", 2: "01", 3: "00"}[pc])


# ------------------------------------------------------------------ headers / container
def image_header(bw, w, h, overlap, out_cf, alpha=0):
    for ch in b"WMPHOTO\0":
        bw.put(ch, 8)
    bw.put(1, 4)            # version
    bw.put(0, 1)            # hard tiling
    bw.put(1, 3)            # sub-version
    bw.put(0, 1)            # tiling
    bw.put(0, 1)            # frequency mode
    bw.put(0, 3)            # spatial transform
    bw.put(0, 1)            # index table
    bw.put(overlap, 2)
    bw.put(1, 1)            # short header
    bw.put(1, 1)            # long word
    bw.put(0, 1)            # windowing
    bw.put(0, 1)            # trim flexbits
    bw.put(0, 1)            # reserved
    bw.put(0, 1)            # red / blue not swapped
    bw.put(0, 1)            # premultiplied
    bw.put(alpha, 1)        # interleaved alpha plane
    bw.put(out_cf, 4)
    bw.put(1, 4)            # BD8
    bw.put(w - 1, 16)
    bw.put(h - 1, 16)


def put_qp(bw, q, nch):
    if nch == 1:
        bw.put(q[0], 8)
        return
    if q[0] == q[1] == q[2]:
        bw.put(0, 2)
        bw.put(q[0], 8)
    elif q[1] == q[2]:
        bw.put(1, 2)
        bw.put(q[0], 8)
        bw.put(q[1], 8)
    else:
        bw.put(2, 2)
        for v in q:
            bw.put(v, 8)


def plane_header(bw, cf, scaled, qdc, qhp, nch):
    bw.put(cf, 3)
    bw.put(scaled, 1)
    bw.put(0, 4)            # all bands
    if cf == YUV420:
        bw.put(0, 1)
        bw.put(2, 3)        # CHROMA_CENTERING_X: centred (as WIC)
        bw.put(0, 1)
        bw.put(2, 3)        # CHROMA_CENTERING_Y
    elif cf == YUV422:
        bw.put(0, 1)
        bw.put(2, 3)
        bw.put(0, 4)
    elif cf == YUV444:
        bw.put(0, 4)
        bw.put(0, 4)
    bw.put(1, 1)            # DC_IMAGE_PLANE_UNIFORM_FLAG
    put_qp(bw, qdc, nch)
    bw.put(0, 1)            # RESERVED_I_BIT
    bw.put(1, 1)            # LP_IMAGE_PLANE_UNIFORM_FLAG (LP QP = DC QP)
    put_qp(bw, qdc, nch)
    bw.put(0, 1)            # RESERVED_J_BIT
    bw.put(1, 1)            # HP_IMAGE_PLANE_UNIFORM_FLAG
    put_qp(bw, qhp, nch)
    bw.align()


def extend(a, H, W):
    h, w = a.shape
    return np.pad(a, ((0, H - h), (0, W - w)), mode="edge")


def downsample(c, cf):
    """centred chroma decimation for 4:2:2 / 4:2:0: [1 3 3 1] / 8 per subsampled axis (output k
    between samples 2k and 2k + 1), edges replicated"""
    a = c.astype(np.int64)
    if cf in (YUV420, YUV422):
        p = np.pad(a, ((0, 0), (1, 1)), mode="edge")
        a = (p[:, 0:-2:2] + 3 * p[:, 1:-1:2] + 3 * p[:, 2::2] + p[:, 3::2] + 4) >> 3
    if cf == YUV420:
        p = np.pad(a, ((1, 1), (0, 0)), mode="edge")
        a = (p[0:-2:2, :] + 3 * p[1:-1:2, :] + 3 * p[2::2, :] + p[3::2, :] + 4) >> 3
    return a


def codestream(planes, cf, w, h, lossless, qp6, out_cf, log=None):
    """planes: channel arrays (int64, already extended to MB multiples, bias removed, shifted
    and colour converted; chroma already decimated)"""
    nch = len(planes)
    scaled = 0 if lossless else 1
    overlap = 0 if lossless else 1
    if lossless:
        qdc = qhp = [0] * nch
    elif nch == 1:
        qdc, qhp = [qp6[0]], [qp6[3]]
    else:
        qdc, qhp = list(qp6[0:3]), list(qp6[3:6])
    bw = BitWriter()
    image_header(bw, w, h, overlap, out_cf)
    plane_header(bw, cf, scaled, qdc, qhp, nch)
    bw.put(0, 8)            # subsequent bytes (VLW_ESC 0)
    bw.put(0, 8)
    bw.put(1, 24)           # packet header
    bw.put(0, 8)
    P = PlaneEnc([p.tolist() for p in planes], cf, scaled, overlap, None)
    shifts_dc = [1] + [(0 if scaled else 1)] * (nch - 1)
    P.qdc = [qp_value(qdc[c], scaled, shifts_dc[c] if scaled else 0) for c in range(nch)]
    P.qlp = list(P.qdc)
    P.qhp = [qp_value(qhp[c], scaled, 1 if scaled else 0) for c in range(nch)]
    if cf == YONLY:
        P.qdc = [qp_value(qdc[0], scaled, 1 if scaled else 0)]
        P.qlp = list(P.qdc)
    P.transform()
    P.encode(bw, log)
    bw.align()
    return bw.data()


GUID_BGR24 = bytes.fromhex("24c3dd6f034efe4bb1853d77768dc90c")
GUID_BGRA32 = bytes.fromhex("24c3dd6f034efe4bb1853d77768dc90f")


def container(code, guid, w, h, alpha=None, icc=None, xmp=None, dpi=None):
    """dpi: (x, y) 16.16 fixed point (IMGMETA IM_DPIX / IM_DPIY) -> FLOAT resolution tags"""
    tags = [(0xBC01, 1, 16, guid), (0xBC80, 4, 1, w), (0xBC81, 4, 1, h),
            (0xBCC0, 4, 1, None), (0xBCC1, 4, 1, len(code))]
    if dpi and dpi[0] and dpi[1]:
        for tag, v in ((0xBC82, dpi[0]), (0xBC83, dpi[1])):
            tags.append((tag, 11, 1, struct.unpack("<I", struct.pack("<f", v / 65536.0))[0]))
    if alpha is not None:
        tags += [(0xBCC2, 4, 1, None), (0xBCC3, 4, 1, len(alpha))]
    if icc:
        tags.append((0x8773, 7, len(icc), icc))
    if xmp:
        tags.append((0x02BC, 1, len(xmp), xmp))
    tags.sort(key=lambda t: t[0])
    n = len(tags)
    data_off = 8 + 2 + 12 * n + 4
    blobs = bytearray()
    for tag, typ, cnt, val in tags:
        if isinstance(val, bytes) and len(val) > 4:
            blobs += val
            if len(blobs) & 1:
                blobs += b"\0"
    img_off = data_off + len(blobs)
    ents = []
    boff = data_off
    for tag, typ, cnt, val in tags:
        if tag == 0xBCC0:
            val = img_off
        elif tag == 0xBCC2:
            val = img_off + len(code)
        if isinstance(val, bytes):
            if len(val) > 4:
                ents.append(struct.pack("<HHII", tag, typ, cnt, boff))
                boff += len(val) + (len(val) & 1)
            else:
                ents.append(struct.pack("<HHI", tag, typ, cnt) + val.ljust(4, b"\0"))
        else:
            ents.append(struct.pack("<HHII", tag, typ, cnt, val))
    out = b"II\xbc\x01" + struct.pack("<IH", 8, n) + b"".join(ents) + b"\0\0\0\0"
    return out + bytes(blobs) + code + (alpha or b"")


def encode(bgra, quality=90, lossless=False, chroma=444, alpha=None, log=None, icc=None, xmp=None,
           dpi=None):
    bgra = np.asarray(bgra, np.uint8)
    h, w = bgra.shape[:2]
    if alpha is None:
        alpha = bool((bgra[:, :, 3] != 255).any())
    H, W = (h + 15) // 16 * 16, (w + 15) // 16 * 16
    cf = {444: YUV444, 422: YUV422, 420: YUV420}[chroma]
    if lossless:
        cf = YUV444
    sh = 0 if lossless else 3
    b = (extend(bgra[:, :, 0].astype(np.int64), H, W) - 128) << sh
    g = (extend(bgra[:, :, 1].astype(np.int64), H, W) - 128) << sh
    r = (extend(bgra[:, :, 2].astype(np.int64), H, W) - 128) << sh
    y, u, v = fwd_color(r, g, b)
    if cf != YUV444:
        u, v = downsample(u, cf), downsample(v, cf)
    qp6 = None if lossless else qp_indices(quality, chroma)
    code = codestream([y, u, v], cf, w, h, lossless, qp6, 7, log)
    acode = None
    if alpha:
        a = extend(bgra[:, :, 3].astype(np.int64), H, W) - 128
        acode = codestream([a], YONLY, w, h, True, None, 0)
    return container(code, GUID_BGRA32 if alpha else GUID_BGR24, w, h, acode, icc, xmp, dpi)


if __name__ == "__main__":
    import sys
    rng = np.random.default_rng(1)
    img = rng.integers(0, 256, (37, 50, 4), dtype=np.uint8)
    img[:, :, 3] = 255
    out = encode(img, lossless=True)
    open(sys.argv[1] if len(sys.argv) > 1 else "jxr_ref_test.jxr", "wb").write(out)
    print(len(out), "bytes")
