#!/usr/bin/env python3
"""Reference HEVC intra encoder -- the model of ASMLIB/src/img/hevc_enc.s (byte-exact).

One picture, one I slice, IDR.  Fixed structure: CTB 32, coding blocks 8..32 (rate-distortion
split decision), transform block = coding block, PART_2Nx2N only, chroma mode DM (4), no SAO,
deblocking on, no scaling lists / transform skip / sign hiding / RDOQ.  4:0:0 / 4:2:0 / 4:4:4,
8 bits.  Lossless: cu_transquant_bypass for every CU.

Decisions (all integer, ties keep the first candidate):
  * luma modes by SATD: planar, DC and the even angular modes, then the best angular +-1;
    cost = SATD << 15 + (lambda_sad_q8 * mode_bits_q15 >> 8); the 2 cheapest + MPM 0 are
    encoded for real (reconstruction, estimated CABAC bits with exact context tracking) with
    J = SSE << 15 + (lambda_q8 * bits_q15 >> 8), SSE over luma and chroma
  * coding quadtree: J(no split) against J(split flag + 4 sub-blocks), recursively
  lambda = 0.57 * 2^((QP - 12) / 3) (HM intra), lambda_sad = sqrt(lambda); tables below.
Quantisation (HM): level = (|c| * scale[qp % 6] + (171 << (qbits - 9))) >> qbits,
qbits = 14 + qp / 6 + 15 - 8 - log2(n); forward DCT with shifts log2(n) - 1 and log2(n) + 6.
Prediction and reconstruction: hevc_ref.Decoder (intra_pred, reconstruct_residual), so the
reconstruction equals any decoder's output before deblocking.

API
  encode(planes, chroma, qp=27, lossless=False, vui=(1, 13, 6, 1)) -> Result
      planes: lists of rows (u8 values), chroma 0 / 1 / 3; width / height even for 4:2:0
  Result: vps, sps, pps, slice (NAL units with header, emulation prevention), recon (planes as
  coded, cropped), stats
"""

import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import hevc_ref as R  # noqa: E402

LOG2_CTB = 5
LOG2_MIN_CB = 3
SUB = {0: (1, 1), 1: (2, 2), 3: (1, 1)}
QSCALE = [26214, 23302, 20560, 18396, 16384, 14564]
LAM_Q8 = [int(0.57 * 2 ** ((q - 12) / 3) * 256 + 0.5) for q in range(52)]
LAMSAD_Q8 = [int(math.sqrt(0.57 * 2 ** ((q - 12) / 3)) * 256 + 0.5) for q in range(52)]


def _ebits():
    a = (0.01875 / 0.5) ** (1 / 63)
    lps, mps = [], []
    for s in range(64):
        p = 0.5 * a ** s if s < 63 else 0.5 * a ** 62
        lps.append(int(-math.log2(p) * 32768 + 0.5))
        mps.append(int(-math.log2(1 - p) * 32768 + 0.5))
    return lps, mps


EBITS_LPS, EBITS_MPS = _ebits()


# ================================================================== bitstream writing
class BitWriter:
    def __init__(self):
        self.bits = []

    def u(self, n, v):
        for i in range(n - 1, -1, -1):
            self.bits.append((v >> i) & 1)

    def ue(self, v):
        v += 1
        n = v.bit_length()
        self.u(n - 1, 0)
        self.u(n, v)

    def se(self, v):
        self.ue(2 * v - 1 if v > 0 else -2 * v)

    def trailing(self):
        self.bits.append(1)
        while len(self.bits) & 7:
            self.bits.append(0)

    def data(self):
        assert len(self.bits) % 8 == 0
        return bytes(int("".join(map(str, self.bits[i:i + 8])), 2) for i in range(0, len(self.bits), 8))


def nal(t, rbsp):
    out = bytearray([t << 1, 1])
    z = 0
    for b in rbsp:
        if z >= 2 and b <= 3:
            out.append(3)
            z = 0
        out.append(b)
        z = z + 1 if b == 0 else 0
    return bytes(out)


def profile_tier_level(w, chroma):
    """general PTL (no sub-layers): Main for 4:2:0, RExt (4) Main 4:4:4 / Monochrome otherwise."""
    if chroma == 1:
        prof, compat = 1, (1 << 30) | (1 << 29)              # flags for profiles 1 and 2
        cons = [1, 0, 0, 1]                                   # progressive, interlaced, non_packed, frame_only
        ext = 0
    else:
        prof, compat = 4, 1 << 27
        cons = [1, 0, 0, 1]
        # max_12bit max_10bit max_8bit max_422 max_420 max_mono intra one_pic lower_bit_rate
        f = [1, 1, 1, 0, 0, 0, 0, 0, 1] if chroma == 3 else [1, 1, 1, 1, 1, 1, 0, 0, 1]
        ext = int("".join(map(str, f)), 2)
    w.u(2, 0)
    w.u(1, 0)
    w.u(5, prof)
    w.u(32, compat)
    for b in cons:
        w.u(1, b)
    w.u(9, ext)
    w.u(34, 0)
    w.u(1, 0)                                                 # general_inbld / reserved
    w.u(8, 186)                                               # level 6.2


class Cfg:
    pass


def write_vps(cfg):
    w = BitWriter()
    w.u(4, 0)
    w.u(1, 1)
    w.u(1, 1)
    w.u(6, 0)
    w.u(3, 0)
    w.u(1, 1)
    w.u(16, 0xFFFF)
    profile_tier_level(w, cfg.chroma)
    w.u(1, 1)                                                 # sub_layer_ordering_info_present
    w.ue(0)
    w.ue(0)
    w.ue(0)
    w.u(6, 0)                                                 # vps_max_layer_id
    w.ue(0)                                                   # vps_num_layer_sets_minus1
    w.u(1, 0)                                                 # vps_timing_info_present
    w.u(1, 0)                                                 # vps_extension
    w.trailing()
    return nal(32, w.data())


def write_sps(cfg):
    w = BitWriter()
    w.u(4, 0)
    w.u(3, 0)
    w.u(1, 1)
    profile_tier_level(w, cfg.chroma)
    w.ue(0)
    w.ue(cfg.chroma)
    if cfg.chroma == 3:
        w.u(1, 0)
    w.ue(cfg.W)
    w.ue(cfg.H)
    sw, sh = SUB[cfg.chroma]
    cr, cb = (cfg.W - cfg.w0) // sw, (cfg.H - cfg.h0) // sh
    if cr or cb:
        w.u(1, 1)
        w.ue(0)
        w.ue(cr)
        w.ue(0)
        w.ue(cb)
    else:
        w.u(1, 0)
    w.ue(0)
    w.ue(0)
    w.ue(4)                                                   # log2_max_pic_order_cnt_lsb_minus4
    w.u(1, 1)
    w.ue(0)
    w.ue(0)
    w.ue(0)
    w.ue(LOG2_MIN_CB - 3)
    w.ue(LOG2_CTB - LOG2_MIN_CB)
    w.ue(0)                                                   # log2_min_tb - 2
    w.ue(3)                                                   # max TB 32
    w.ue(0)
    w.ue(0)
    w.u(1, 0)                                                 # scaling lists
    w.u(1, 0)                                                 # amp
    w.u(1, 0)                                                 # sao
    w.u(1, 0)                                                 # pcm
    w.ue(0)
    w.u(1, 0)
    w.u(1, 0)
    w.u(1, 1)                                                 # strong intra smoothing
    w.u(1, 1)                                                 # vui
    w.u(1, 0)
    w.u(1, 0)
    w.u(1, 1)                                                 # video_signal_type_present
    w.u(3, 5)
    w.u(1, cfg.vui[3])
    w.u(1, 1)
    w.u(8, cfg.vui[0])
    w.u(8, cfg.vui[1])
    w.u(8, cfg.vui[2])
    w.u(1, 0)                                                 # chroma_loc
    w.u(1, 0)
    w.u(1, 0)
    w.u(1, 0)
    w.u(1, 0)                                                 # default display window
    w.u(1, 0)                                                 # timing
    w.u(1, 0)                                                 # bitstream restriction
    w.u(1, 0)                                                 # sps_extension
    w.trailing()
    return nal(33, w.data())


def write_pps(cfg):
    w = BitWriter()
    w.ue(0)
    w.ue(0)
    w.u(1, 0)
    w.u(1, 0)
    w.u(3, 0)
    w.u(1, 0)                                                 # sign data hiding
    w.u(1, 0)
    w.ue(0)
    w.ue(0)
    w.se(cfg.qp - 26)
    w.u(1, 0)
    w.u(1, 0)                                                 # transform skip
    w.u(1, 0)                                                 # cu_qp_delta
    w.se(0)
    w.se(0)
    w.u(1, 0)
    w.u(1, 0)
    w.u(1, 0)
    w.u(1, 1 if cfg.lossless else 0)                          # transquant_bypass_enabled
    w.u(1, 0)                                                 # tiles
    w.u(1, 0)                                                 # entropy coding sync
    w.u(1, 0)                                                 # loop filter across slices
    if cfg.deblock:
        w.u(1, 0)                                             # deblocking_filter_control_present
    else:
        w.u(1, 1)
        w.u(1, 0)                                             # override enabled
        w.u(1, 1)                                             # pps_deblocking_filter_disabled
    w.u(1, 0)
    w.u(1, 0)
    w.ue(0)
    w.u(1, 0)
    w.u(1, 0)
    w.trailing()
    return nal(34, w.data())


def slice_header(w):
    w.u(1, 1)                                                 # first_slice_segment_in_pic
    w.u(1, 0)                                                 # no_output_of_prior_pics
    w.ue(0)
    w.ue(2)                                                   # I
    w.se(0)                                                   # slice_qp_delta
    w.trailing()                                              # byte_alignment()


# ================================================================== CABAC
class CabacWriter:
    """Arithmetic encoder of H.265 9.3.4.3 (informative): PutBit with bitsOutstanding."""

    def __init__(self, w):
        self.w = w
        self.low, self.range = 0, 510
        self.first, self.outstanding = 1, 0

    def put(self, b):
        if self.first:
            self.first = 0
        else:
            self.w.bits.append(b)
        while self.outstanding:
            self.w.bits.append(1 - b)
            self.outstanding -= 1

    def renorm(self):
        while self.range < 256:
            if self.low < 256:
                self.put(0)
            elif self.low >= 512:
                self.low -= 512
                self.put(1)
            else:
                self.low -= 256
                self.outstanding += 1
            self.range <<= 1
            self.low <<= 1

    def bin(self, ctx, b):
        st, mps = ctx
        lps = R.LPS_TAB[st][(self.range >> 6) & 3]
        self.range -= lps
        if b != mps:
            self.low += self.range
            self.range = lps
            if st == 0:
                mps = 1 - mps
            st = R.TRANS_LPS[st]
        else:
            st = R.TRANS_MPS[st]
        ctx[0], ctx[1] = st, mps
        self.renorm()

    def bypass(self, b):
        self.low <<= 1
        if b:
            self.low += self.range
        if self.low >= 1024:
            self.put(1)
            self.low -= 1024
        elif self.low < 512:
            self.put(0)
        else:
            self.low -= 512
            self.outstanding += 1

    def bypass_bits(self, v, n):
        for i in range(n - 1, -1, -1):
            self.bypass((v >> i) & 1)

    def terminate(self, b):
        self.range -= 2
        if b:
            self.low += self.range
            self.range = 2
            self.renorm()
            self.put((self.low >> 9) & 1)
            v = ((self.low >> 7) & 3) | 1
            self.w.bits.append(v >> 1)
            self.w.bits.append(v & 1)                         # = rbsp_stop_one_bit
        else:
            self.renorm()


class CabacEst:
    """Bit estimator (Q15) with exact context state tracking."""

    def __init__(self):
        self.bits = 0

    def bin(self, ctx, b):
        st, mps = ctx
        if b != mps:
            self.bits += EBITS_LPS[st]
            if st == 0:
                mps = 1 - mps
            st = R.TRANS_LPS[st]
        else:
            self.bits += EBITS_MPS[st]
            st = R.TRANS_MPS[st]
        ctx[0], ctx[1] = st, mps

    def bypass(self, b):
        self.bits += 32768

    def bypass_bits(self, v, n):
        self.bits += 32768 * n


# ================================================================== transforms
def fdct(res, n, bd=8):
    m = R.DCT[n]
    log2 = n.bit_length() - 1
    s1, s2 = log2 + bd - 9, log2 + 6
    tmp = [[(sum(m[k][x] * res[y][x] for x in range(n)) + (1 << (s1 - 1))) >> s1 for k in range(n)]
           for y in range(n)]
    return [[(sum(m[k][y] * tmp[y][kx] for y in range(n)) + (1 << (s2 - 1))) >> s2 for kx in range(n)]
            for k in range(n)]


def quant(coef, n, qp, bd=8):
    log2 = n.bit_length() - 1
    qbits = 14 + qp // 6 + 15 - bd - log2
    add = 171 << (qbits - 9)
    sc = QSCALE[qp % 6]
    out = [[0] * n for _ in range(n)]
    for y in range(n):
        for x in range(n):
            c = coef[y][x]
            lv = (abs(c) * sc + add) >> qbits
            out[y][x] = -lv if c < 0 else lv
    return out


def hadamard(blk):
    """unnormalised Walsh-Hadamard of a square block (rows, then columns); sum of |values|"""
    n = len(blk)
    a = [row[:] for row in blk]
    h = 1
    while h < n:
        for r in a:
            for i in range(0, n, 2 * h):
                for j in range(i, i + h):
                    r[j], r[j + h] = r[j] + r[j + h], r[j] - r[j + h]
        h *= 2
    h = 1
    while h < n:
        for i in range(0, n, 2 * h):
            for j in range(i, i + h):
                for x in range(n):
                    a[j][x], a[j + h][x] = a[j][x] + a[j + h][x], a[j][x] - a[j + h][x]
        h *= 2
    return sum(abs(v) for r in a for v in r)


def satd(res, n):
    if n == 4:
        return (hadamard(res) + 1) >> 1
    t = 0
    for by in range(0, n, 8):
        for bx in range(0, n, 8):
            t += (hadamard([row[bx:bx + 8] for row in res[by:by + 8]]) + 2) >> 2
    return t


# ================================================================== encoder
class Leaf:
    __slots__ = ("x", "y", "log2", "depth", "mode", "mpm", "coef", "cbf")


class Encoder:
    def __init__(self, planes, chroma, qp, lossless, vui, deblock=True):
        cfg = Cfg()
        cfg.chroma, cfg.qp, cfg.lossless, cfg.vui, cfg.deblock = chroma, qp, lossless, vui, deblock
        cfg.h0, cfg.w0 = len(planes[0]), len(planes[0][0])
        cfg.W, cfg.H = -(-cfg.w0 // 8) * 8, -(-cfg.h0 // 8) * 8
        self.cfg = cfg
        sw, sh = SUB[chroma]
        # source padded by edge replication
        self.src = []
        for c, p in enumerate(planes[:1 if chroma == 0 else 3]):
            pw = cfg.W if c == 0 else cfg.W // sw
            ph = cfg.H if c == 0 else cfg.H // sh
            rows = [list(r) + [r[-1]] * (pw - len(r)) for r in p]
            rows += [rows[-1][:] for _ in range(ph - len(rows))]
            self.src.append(rows)
        self.vps, self.sps, self.pps = write_vps(cfg), write_sps(cfg), write_pps(cfg)
        d = R.Decoder()
        s = R.parse_sps(R.unescape(self.sps[2:]))
        p = R.parse_pps(R.unescape(self.pps[2:]), {0: s})
        d.new_picture(s, p)
        d.sh = type("SH", (), {})()
        d.sh.cb_qp_offset = d.sh.cr_qp_offset = 0
        d.slice_addr, d.cur_tile = 0, 0
        d.cu_qpy, d.cu_bypass = qp, 1 if lossless else 0
        self.d = d
        self.ncomp = 1 if chroma == 0 else 3

    # -------------------------------------------------------------- state snapshots
    def region(self, x, y, log2):
        sw, sh = SUB[self.cfg.chroma]
        n = 1 << log2
        out = [(0, x, y, n, n)]
        for c in range(1, self.ncomp):
            out.append((c, x // sw, y // sh, n // sw, n // sh))
        return out

    def save(self, x, y, log2, ctx):
        d = self.d
        n = 1 << log2
        planes = [[row[cx:cx + w] for row in d.pic.planes[c][cy:cy + h]] for c, cx, cy, w, h in self.region(x, y, log2)]
        maps = [[m[yy][x >> 2:(x + n) >> 2] for yy in range(y >> 2, (y + n) >> 2)]
                for m in (d.dec, d.tid, d.ipm, d.ctdepth)]
        return planes, maps, R.copy_contexts(ctx)

    def restore(self, snap, x, y, log2, ctx):
        d = self.d
        n = 1 << log2
        planes, maps, cx = snap
        for (c, px, py, w, h), blk in zip(self.region(x, y, log2), planes):
            for j, row in enumerate(blk):
                d.pic.planes[c][py + j][px:px + w] = row
        for m, blk in zip((d.dec, d.tid, d.ipm, d.ctdepth), maps):
            for j, row in enumerate(blk):
                m[(y >> 2) + j][x >> 2:(x + n) >> 2] = row
        for k in ctx:
            for a, b in zip(ctx[k], cx[k]):
                a[0], a[1] = b

    # -------------------------------------------------------------- syntax
    def split_ctx(self, x, y, depth):
        d = self.d
        inc = 0
        if d.available(x, y, x - 1, y) and d.ctdepth[y >> 2][(x - 1) >> 2] > depth:
            inc += 1
        if d.available(x, y, x, y - 1) and d.ctdepth[(y - 1) >> 2][x >> 2] > depth:
            inc += 1
        return inc

    def code_mode(self, B, ctx, mode, mpm):
        if mode in mpm:
            B.bin(ctx["prev_intra"][0], 1)
            i = mpm.index(mode)
            B.bypass(1 if i > 0 else 0)
            if i > 0:
                B.bypass(1 if i > 1 else 0)
        else:
            B.bin(ctx["prev_intra"][0], 0)
            B.bypass_bits(mode - sum(1 for m in mpm if m < mode), 5)

    def residual(self, B, ctx, c, log2, coef, mode):
        """residual_coding() of a TB (mirror of hevc_ref.Decoder.residual)."""
        cat = self.cfg.chroma
        n = 1 << log2
        scan_idx = 0
        if log2 == 2 or (log2 == 3 and (c == 0 or cat == 3)):
            if 6 <= mode <= 14:
                scan_idx = 2
            elif 22 <= mode <= 30:
                scan_idx = 1
        nsb = 1 << (log2 - 2)
        sb_scan = R.SCAN[nsb][scan_idx] if nsb > 1 else [(0, 0)]
        pos_scan = R.SCAN4[scan_idx]
        # last significant coefficient in scan order
        last_sb = last_pos = -1
        for i in range(len(sb_scan) - 1, -1, -1):
            xs, ys = sb_scan[i]
            for k in range(15, -1, -1):
                xp, yp = pos_scan[k]
                if coef[(ys << 2) + yp][(xs << 2) + xp]:
                    last_sb, last_pos = i, k
                    break
            if last_sb >= 0:
                break
        xs, ys = sb_scan[last_sb]
        xp, yp = pos_scan[last_pos]
        lx, ly = (xs << 2) + xp, (ys << 2) + yp
        if scan_idx == 2:
            lx, ly = ly, lx
        if c == 0:
            off, shift = 3 * (log2 - 2) + ((log2 - 1) >> 2), (log2 + 1) >> 2
        else:
            off, shift = 15, log2 - 2
        cmax = (log2 << 1) - 1

        def prefix_of(v):
            if v < 4:
                return v
            k = v.bit_length() - 1          # v in [2^k, 2^(k+1))
            return 2 * k + (1 if v >= 3 << (k - 1) else 0)
        px, py = prefix_of(lx), prefix_of(ly)
        for pfx, name in ((px, "last_x"), (py, "last_y")):
            for i in range(pfx):
                B.bin(ctx[name][off + (i >> shift)], 1)
            if pfx < cmax:
                B.bin(ctx[name][off + (pfx >> shift)], 0)
        for pfx, v in ((px, lx), (py, ly)):
            if pfx > 3:
                nb = (pfx >> 1) - 1
                B.bypass_bits(v - ((1 << nb) * (2 + (pfx & 1))), nb)
        csbf = {}
        prev_g1_state = None
        for i in range(last_sb, -1, -1):
            xs, ys = sb_scan[i]
            vals = []
            for k in range(16):
                xp, yp = pos_scan[k]
                vals.append(coef[(ys << 2) + yp][(xs << 2) + xp])
            infer_dc = 0
            if 0 < i < last_sb:
                right = csbf.get((xs + 1, ys), 0) if xs + 1 < nsb else 0
                below = csbf.get((xs, ys + 1), 0) if ys + 1 < nsb else 0
                inc = min(right + below, 1) + (2 if c else 0)
                flag = 1 if any(vals) else 0
                B.bin(ctx["csbf"][inc], flag)
                csbf[(xs, ys)] = flag
                infer_dc = 1
            else:
                csbf[(xs, ys)] = 1
            sig = []
            start = last_pos - 1 if i == last_sb else 15
            if i == last_sb:
                sig.append(last_pos)
            right = csbf.get((xs + 1, ys), 0) if xs + 1 < nsb else 0
            below = csbf.get((xs, ys + 1), 0) if ys + 1 < nsb else 0
            prev_csbf = right | (below << 1)
            if csbf[(xs, ys)]:
                for k in range(start, -1, -1):
                    xp, yp = pos_scan[k]
                    xc, yc = (xs << 2) + xp, (ys << 2) + yp
                    f = 1 if vals[k] else 0
                    if k > 0 or not infer_dc:
                        if log2 == 2:
                            sctx = R.CTX_IDX_MAP_4x4[(yc << 2) + xc]
                        elif xc + yc == 0:
                            sctx = 0
                        else:
                            if prev_csbf == 0:
                                sctx = 2 if xp + yp == 0 else (1 if xp + yp < 3 else 0)
                            elif prev_csbf == 1:
                                sctx = 2 if yp == 0 else (1 if yp == 1 else 0)
                            elif prev_csbf == 2:
                                sctx = 2 if xp == 0 else (1 if xp == 1 else 0)
                            else:
                                sctx = 2
                            if c == 0:
                                if (xs, ys) != (0, 0):
                                    sctx += 3
                                sctx += (9 if scan_idx == 0 else 15) if log2 == 3 else 21
                            else:
                                sctx += 9 if log2 == 3 else 12
                        sctx = sctx if c == 0 else 27 + sctx
                        B.bin(ctx["sig"][sctx], f)
                        if f:
                            sig.append(k)
                            infer_dc = 0
                    else:
                        sig.append(0)
            if not sig:
                continue
            absl = [abs(vals[k]) for k in sig]
            ctx_set = 0 if (i == 0 or c > 0) else 2
            if prev_g1_state is not None and prev_g1_state == 0:
                ctx_set += 1
            g1ctx = 1
            first_g2 = -1
            for j in range(min(8, len(sig))):
                inc = ctx_set * 4 + g1ctx + (16 if c else 0)
                f = 1 if absl[j] > 1 else 0
                B.bin(ctx["gt1"][inc], f)
                if f:
                    g1ctx = 0
                    if first_g2 < 0:
                        first_g2 = j
                elif 0 < g1ctx < 3:
                    g1ctx += 1
            prev_g1_state = g1ctx
            if first_g2 >= 0:
                B.bin(ctx["gt2"][ctx_set + (4 if c else 0)], 1 if absl[first_g2] > 2 else 0)
            for j, k in enumerate(sig):
                B.bypass(1 if vals[k] < 0 else 0)
            rice = 0
            first_coeff2 = 1
            for j in range(len(sig)):
                a = absl[j]
                if j < 8:
                    partial = 1 + (1 if a > 1 else 0) + (1 if j == first_g2 and a > 2 else 0)
                    base = 2 + first_coeff2
                else:
                    partial = base = 1
                if partial == base:
                    self.write_rem(B, a - base, rice)
                    if a > 3 * (1 << rice):
                        rice = min(rice + 1, 4)
                if a >= 2:
                    first_coeff2 = 0

    @staticmethod
    def write_rem(B, rem, rice):
        """coeff_abs_level_remaining: prefix (unary) + suffix, inverse of hevc_ref's reader"""
        if rem < (4 << rice):
            prefix = rem >> rice
            B.bypass_bits((1 << (prefix + 1)) - 2, prefix + 1)       # prefix ones and a zero
            B.bypass_bits(rem & ((1 << rice) - 1), rice)
        else:
            # rem = (((1 << (prefix - 3)) + 2) << rice) + suffix, suffix of prefix - 3 + rice bits
            prefix = 4
            while rem >= (((1 << (prefix - 2)) + 2) << rice):
                prefix += 1
            B.bypass_bits((1 << (prefix + 1)) - 2, prefix + 1)
            B.bypass_bits(rem - ((((1 << (prefix - 3)) + 2)) << rice), prefix - 3 + rice)

    # -------------------------------------------------------------- one coding unit
    def code_cu(self, B, ctx, lf):
        """syntax of a leaf CU (after split_cu_flag)"""
        if self.cfg.lossless:
            B.bin(ctx["tq_bypass"][0], 1)
        if lf.log2 == LOG2_MIN_CB:
            B.bin(ctx["part_mode"][0], 1)
        self.code_mode(B, ctx, lf.mode, lf.mpm)
        if self.ncomp > 1:
            B.bin(ctx["chroma_mode"][0], 0)
            B.bin(ctx["cbf_chroma"][0], lf.cbf[1])
            B.bin(ctx["cbf_chroma"][0], lf.cbf[2])
        B.bin(ctx["cbf_luma"][1], lf.cbf[0])
        sw, sh = SUB[self.cfg.chroma]
        for c in range(self.ncomp):
            if lf.cbf[c]:
                l2 = lf.log2 if c == 0 or sw == 1 else lf.log2 - 1
                self.residual(B, ctx, c, l2, lf.coef[c], lf.mode)

    def encode_leaf(self, B, ctx, x, y, log2, depth, mode, mpm):
        """predict / transform / quantise / reconstruct with the given luma mode; syntax into B;
        returns (Leaf, SSE)"""
        d = self.d
        cfg = self.cfg
        n = 1 << log2
        d.cu_x, d.cu_y, d.cu_log2 = x, y, log2
        d.set_cu_info(x, y, n, depth, [mode], False, d.cu_bypass)
        for yy in range(y, y + n, 4):
            for xx in range(x, x + n, 4):
                d.ipm[yy >> 2][xx >> 2] = mode
        lf = Leaf()
        lf.x, lf.y, lf.log2, lf.depth, lf.mode, lf.mpm = x, y, log2, depth, mode, mpm
        lf.coef, lf.cbf = [None] * 3, [0, 0, 0]
        sse = 0
        sw, sh = SUB[cfg.chroma]
        for c in range(self.ncomp):
            cx, cy = (x, y) if c == 0 else (x // sw, y // sh)
            l2 = log2 if c == 0 or sw == 1 else log2 - 1
            m = 1 << l2
            d.intra_pred(c, cx, cy, l2, mode)
            pl, sp = d.pic.planes[c], self.src[c]
            res = [[sp[cy + j][cx + i] - pl[cy + j][cx + i] for i in range(m)] for j in range(m)]
            if cfg.lossless:
                coef = res
            else:
                qp = cfg.qp if c == 0 else self.chroma_qp()
                coef = quant(fdct(res, m), m, qp)
            if any(v for row in coef for v in row):
                lf.cbf[c] = 1
                lf.coef[c] = coef
                d.reconstruct_residual(c, cx, cy, l2, coef, 0, mode)
            if c == 0:
                d.mark_decoded(x, y, n, n)
            for j in range(m):
                for i in range(m):
                    e = sp[cy + j][cx + i] - pl[cy + j][cx + i]
                    sse += e * e
        self.code_cu(B, ctx, lf)
        return lf, sse

    def chroma_qp(self):
        qpi = max(0, min(57, self.cfg.qp))
        if self.cfg.chroma == 1:
            return qpi if qpi < 30 else (R.QPC_TAB[qpi - 30] if qpi <= 43 else qpi - 6)
        return min(qpi, 51)

    def mode_bits(self, m, mpm):
        if m == mpm[0]:
            return 2 << 15
        if m in mpm:
            return 3 << 15
        return 6 << 15

    def leaf_search(self, ctx, x, y, log2, depth):
        """best luma mode of a CU: SATD pre-selection, then real coding of up to 3 modes"""
        d = self.d
        cfg = self.cfg
        n = 1 << log2
        d.cu_x, d.cu_y, d.cu_log2 = x, y, log2
        mpm = d.mpm_list(x, y)
        lam_sad = LAMSAD_Q8[cfg.qp]

        def sad_cost(m):
            d.intra_pred(0, x, y, log2, m)
            pl, sp = d.pic.planes[0], self.src[0]
            res = [[sp[y + j][x + i] - pl[y + j][x + i] for i in range(n)] for j in range(n)]
            return (satd(res, n) << 15) + ((lam_sad * self.mode_bits(m, mpm)) >> 8)
        costs = []
        for m in [0, 1] + list(range(2, 35, 2)):
            costs.append((sad_cost(m), m))
        best_ang = min((cm for cm in costs if cm[1] >= 2), key=lambda t: t[0])[1]
        for m in (best_ang - 1, best_ang + 1):
            if 2 <= m <= 34:
                costs.append((sad_cost(m), m))
        order = sorted(range(len(costs)), key=lambda i: (costs[i][0], i))
        cands = [costs[order[0]][1], costs[order[1]][1]]
        if mpm[0] not in cands:
            cands.append(mpm[0])
        snap0 = self.save(x, y, log2, ctx)
        best = None
        for m in cands:
            self.restore(snap0, x, y, log2, ctx)
            est = CabacEst()
            lf, sse = self.encode_leaf(est, ctx, x, y, log2, depth, m, mpm)
            j = (sse << 15) + ((LAM_Q8[cfg.qp] * est.bits) >> 8)
            if best is None or j < best[0]:
                best = (j, lf, self.save(x, y, log2, ctx), est.bits)
        self.restore(best[2], x, y, log2, ctx)
        return best[0], best[1], best[3]

    def cu_search(self, ctx, x, y, log2, depth):
        """returns (J, tree) and leaves the winner's reconstruction / contexts / maps in place;
        tree: Leaf or ('split', [subtrees]); the split flag bins are part of J"""
        cfg = self.cfg
        n = 1 << log2
        if x + n > cfg.W or y + n > cfg.H:
            j, subs = 0, []
            h = n >> 1
            for sy in (0, 1):
                for sx in (0, 1):
                    xx, yy = x + sx * h, y + sy * h
                    if xx < cfg.W and yy < cfg.H:
                        jj, t = self.cu_search(ctx, xx, yy, log2 - 1, depth + 1)
                        j += jj
                        subs.append(t)
            return j, ("split", subs, None)
        can_split = log2 > LOG2_MIN_CB
        snap0 = self.save(x, y, log2, ctx) if can_split else None
        est = CabacEst()
        if can_split:
            est.bin(ctx["split_cu"][self.split_ctx(x, y, depth)], 0)
        ja, lf, _ = self.leaf_search(ctx, x, y, log2, depth)
        ja += (LAM_Q8[cfg.qp] * est.bits) >> 8
        if not can_split:
            return ja, lf
        snap_a = self.save(x, y, log2, ctx)
        self.restore(snap0, x, y, log2, ctx)
        est = CabacEst()
        est.bin(ctx["split_cu"][self.split_ctx(x, y, depth)], 1)
        jb = (LAM_Q8[cfg.qp] * est.bits) >> 8
        subs = []
        h = n >> 1
        for sy in (0, 1):
            for sx in (0, 1):
                jj, t = self.cu_search(ctx, x + sx * h, y + sy * h, log2 - 1, depth + 1)
                jb += jj
                subs.append(t)
        if jb < ja:
            return jb, ("split", subs, 1)
        self.restore(snap_a, x, y, log2, ctx)
        return ja, lf

    def emit(self, B, ctx, tree, x, y, log2, depth):
        cfg = self.cfg
        n = 1 << log2
        inside = x + n <= cfg.W and y + n <= cfg.H
        if isinstance(tree, Leaf):
            if log2 > LOG2_MIN_CB:
                B.bin(ctx["split_cu"][self.split_ctx(x, y, depth)], 0)
            self.code_cu(B, ctx, tree)
            return
        if inside:
            B.bin(ctx["split_cu"][self.split_ctx(x, y, depth)], 1)
        h = n >> 1
        k = 0
        for sy in (0, 1):
            for sx in (0, 1):
                xx, yy = x + sx * h, y + sy * h
                if xx < cfg.W and yy < cfg.H:
                    self.emit(B, ctx, tree[1][k], xx, yy, log2 - 1, depth + 1)
                    k += 1

    def encode(self):
        cfg = self.cfg
        w = BitWriter()
        slice_header(w)
        cab = CabacWriter(w)
        ctx = R.init_contexts(cfg.qp)
        nx, ny = -(-cfg.W // 32), -(-cfg.H // 32)
        for ry in range(ny):
            for rx in range(nx):
                x, y = rx << LOG2_CTB, ry << LOG2_CTB
                sctx = R.copy_contexts(ctx)
                _, tree = self.cu_search(sctx, x, y, LOG2_CTB, 0)
                # the emission needs the depth map as decided: cu_search left it in place
                self.emit(cab, ctx, tree, x, y, LOG2_CTB, 0)
                cab.terminate(1 if (rx == nx - 1 and ry == ny - 1) else 0)
        while len(w.bits) & 7:
            w.bits.append(0)
        self.slice = nal(19, w.data())
        planes = self.d.pic.planes
        sw, sh = SUB[cfg.chroma]
        self.recon = [[row[:(cfg.w0 if c == 0 else cfg.w0 // sw)] for row in planes[c][:(cfg.h0 if c == 0 else cfg.h0 // sh)]]
                      for c in range(self.ncomp)]


class Result:
    pass


def encode(planes, chroma, qp=27, lossless=False, vui=(1, 13, 6, 1), deblock=True):
    """deblock=False writes pps_deblocking_filter_disabled (tests: decoder output == recon)"""
    e = Encoder(planes, chroma, qp, lossless, vui, deblock)
    e.encode()
    r = Result()
    r.vps, r.sps, r.pps, r.slice, r.recon = e.vps, e.sps, e.pps, e.slice, e.recon
    return r


if __name__ == "__main__":
    import random
    rnd = random.Random(1)
    w, h = 48, 40
    Y = [[(x * 3 + y * 2 + rnd.randrange(20)) & 255 for x in range(w)] for y in range(h)]
    U = [[(128 + x - y) & 255 for x in range(w // 2)] for y in range(h // 2)]
    V = [[(100 + 2 * y) & 255 for x in range(w // 2)] for y in range(h // 2)]
    r = encode([Y, U, V], 1, qp=30)
    pic = R.decode([r.vps, r.sps, r.pps, r.slice])
    print(len(r.slice), "bytes slice")
