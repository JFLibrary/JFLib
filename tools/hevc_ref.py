#!/usr/bin/env python3
"""Reference HEVC (ITU-T H.265) intra picture decoder -- the model of ASMLIB/src/img/hevc_*.s.

Scope: what still images (HEIF / HEIC) use: one picture of I slices, Main / Main 10 / Main Still
Picture / Main 4:4:4 (and 4:2:2, 4:0:0) without the RExt coding tools beyond what x265 emits,
8..12 bits, CABAC, coding / transform quadtrees, intra prediction (planar, DC, 33 angular modes,
filtering, strong smoothing), scaling lists, transform skip, transquant bypass, PCM, sign data
hiding, cu_qp_delta, deblocking, SAO, tiles, wavefront (entropy_coding_sync), several slices and
dependent slice segments.  The output is bit-exact to any conforming decoder (checked against
ffmpeg's hevc decoder by tools/heic_check.py).

API
  decode(nals) -> Picture       nals: list of NAL unit payloads (with the 2-byte header, emulation
                                prevention still in) -- VPS / SPS / PPS / slice segments
  split_annexb(data) -> nals    Annex B byte stream (00 00 01 start codes)
  split_lp(data, n) -> nals     length-prefixed NAL units (n = 1..4 bytes, as in hvcC / HEIF)
  Picture: planes [Y, Cb, Cr] (lists of rows, cropped by the conformance window), bit_depth,
  chroma_format_idc, width, height

stdlib only (numpy not needed); slow -- meant for small test pictures.
"""

import sys

# ================================================================== bit reader
class BitReader:
    def __init__(self, data):
        self.d = data
        self.pos = 0

    def u(self, n):
        v = 0
        for _ in range(n):
            v = (v << 1) | ((self.d[self.pos >> 3] >> (7 - (self.pos & 7))) & 1) if (self.pos >> 3) < len(self.d) else (v << 1)
            self.pos += 1
        return v

    def f(self):
        return self.u(1)

    def ue(self):
        lz = 0
        while self.u(1) == 0:
            lz += 1
            if lz > 32:
                raise ValueError("ue")
        return (1 << lz) - 1 + self.u(lz)

    def se(self):
        k = self.ue()
        return (k + 1) >> 1 if k & 1 else -(k >> 1)

    def byte_aligned(self):
        return (self.pos & 7) == 0

    def more_rbsp_data(self):
        # true if there is more data before the rbsp trailing bits
        n = len(self.d)
        last = n - 1
        while last >= 0 and self.d[last] == 0:
            last -= 1
        if last < 0:
            return False
        b = self.d[last]
        tb = last * 8 + 7
        while not (b & 1):
            b >>= 1
            tb -= 1
        return self.pos < tb


def unescape(nal):
    out = bytearray()
    z = 0
    i = 0
    while i < len(nal):
        b = nal[i]
        if z >= 2 and b == 3:
            z = 0
            i += 1
            continue
        out.append(b)
        z = z + 1 if b == 0 else 0
        i += 1
    return bytes(out)


def split_annexb(data):
    nals = []
    i = 0
    n = len(data)
    starts = []
    while i + 3 <= n:
        if data[i] == 0 and data[i + 1] == 0 and data[i + 2] == 1:
            starts.append(i + 3)
            i += 3
        else:
            i += 1
    for k, s in enumerate(starts):
        e = starts[k + 1] - 3 if k + 1 < len(starts) else n
        while e > s and data[e - 1] == 0:
            e -= 1
        nals.append(bytes(data[s:e]))
    return nals


def split_lp(data, n):
    nals = []
    i = 0
    while i + n <= len(data):
        ln = int.from_bytes(data[i:i + n], "big")
        i += n
        nals.append(bytes(data[i:i + ln]))
        i += ln
    return nals


# ================================================================== tables
DEFAULT_SL_INTRA8 = [16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 16, 17, 16, 17, 18, 17, 18, 18, 17, 18, 21, 19, 20,
                     21, 20, 19, 21, 24, 22, 22, 24, 24, 22, 22, 24, 25, 25, 27, 30, 27, 25, 25, 29, 31, 35, 35, 31,
                     29, 36, 41, 44, 41, 36, 47, 54, 54, 47, 65, 70, 65, 88, 88, 115]
DEFAULT_SL_INTER8 = [16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 20, 20, 20,
                     20, 20, 20, 20, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 28, 28, 28, 28, 28,
                     28, 33, 33, 33, 33, 33, 41, 41, 41, 41, 54, 54, 54, 71, 71, 91]

LPS_TAB = [
    [128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205], [116, 142, 169, 195],
    [111, 135, 160, 185], [105, 128, 152, 175], [100, 122, 144, 166], [95, 116, 137, 158], [90, 110, 130, 150],
    [85, 104, 123, 142], [81, 99, 117, 135], [77, 94, 111, 128], [73, 89, 105, 122], [69, 85, 100, 116],
    [66, 80, 95, 110], [62, 76, 90, 104], [59, 72, 86, 99], [56, 69, 81, 94], [53, 65, 77, 89], [51, 62, 73, 85],
    [48, 59, 69, 80], [46, 56, 66, 76], [43, 53, 63, 72], [41, 50, 59, 69], [39, 48, 56, 65], [37, 45, 54, 62],
    [35, 43, 51, 59], [33, 41, 48, 56], [32, 39, 46, 53], [30, 37, 43, 50], [29, 35, 41, 48], [27, 33, 39, 45],
    [26, 31, 37, 43], [24, 30, 35, 41], [23, 28, 33, 39], [22, 27, 32, 37], [21, 26, 30, 35], [20, 24, 29, 33],
    [19, 23, 27, 31], [18, 22, 26, 30], [17, 21, 25, 28], [16, 20, 23, 27], [15, 19, 22, 25], [14, 18, 21, 24],
    [14, 17, 20, 23], [13, 16, 19, 22], [12, 15, 18, 21], [12, 14, 17, 20], [11, 14, 16, 19], [11, 13, 15, 18],
    [10, 12, 15, 17], [10, 12, 14, 16], [9, 11, 13, 15], [9, 11, 12, 14], [8, 10, 12, 14], [8, 9, 11, 13],
    [7, 9, 11, 12], [7, 9, 10, 12], [7, 8, 10, 11], [6, 8, 9, 11], [6, 7, 9, 10], [6, 7, 8, 9], [2, 2, 2, 2]]
TRANS_LPS = [0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12, 13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22,
             23, 24, 24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33, 33, 33, 34, 34, 35, 35, 35, 36,
             36, 36, 37, 37, 37, 38, 38, 63]
TRANS_MPS = [min(i + 1, 62) for i in range(62)] + [62, 63]

# context init values, initType 0 (I slices)
INIT = {
    "sao_merge": [153],
    "sao_type": [200],
    "split_cu": [139, 141, 157],
    "tq_bypass": [154],
    "part_mode": [184],
    "prev_intra": [184],
    "chroma_mode": [63],
    "split_tf": [153, 138, 138],
    "cbf_luma": [111, 141],
    "cbf_chroma": [94, 138, 182, 154, 154],
    "cu_qp_delta": [154, 154],
    "cu_chroma_qp_offset_flag": [154],
    "cu_chroma_qp_offset_idx": [154],
    "ts_flag": [139, 139],
    "last_x": [110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63],
    "last_y": [110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63],
    "csbf": [91, 171, 134, 141],
    "sig": [111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153,
            125, 107, 125, 141, 179, 153, 125, 140, 139, 182, 182, 152, 136, 152, 136, 153, 136, 139, 111, 136,
            139, 111, 141, 111],
    "gt1": [140, 92, 137, 138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107, 122, 152, 140, 179, 166, 182,
            140, 227, 122, 197],
    "gt2": [138, 153, 136, 167, 152, 152],
    "log2_res_scale": [154] * 8,
    "res_scale_sign": [154] * 2,
}
CTX_IDX_MAP_4x4 = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8]

INTRA_ANGLE = [0, 0, 32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, -32, -26, -21, -17, -13, -9,
               -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32]
INV_ANGLE = {11: -4096, 12: -1638, 13: -910, 14: -630, 15: -482, 16: -390, 17: -315, 18: -256, 19: -315, 20: -390,
             21: -482, 22: -630, 23: -910, 24: -1638, 25: -4096}
# 4:2:2 chroma mode mapping (Table 8-3 as corrected: 11 -> 12 and 14 -> 17, the angle doubled horizontally)
MODE_422 = [0, 1, 2, 2, 2, 2, 3, 5, 7, 8, 10, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25, 26, 27,
            27, 28, 28, 29, 29, 30, 31]
QPC_TAB = [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37]
LEVEL_SCALE = [40, 45, 51, 57, 64, 72]
BETA_TAB = [0] * 16 + [6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42,
                       44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64]
TC_TAB = [0] * 18 + [1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13,
                     14, 16, 18, 20, 22, 24]
DST4 = [[29, 55, 74, 84], [74, 74, 0, -74], [84, -29, -74, 55], [55, -84, 74, -29]]
_C32 = [64, 90, 90, 90, 89, 88, 87, 85, 83, 82, 80, 78, 75, 73, 70, 67, 64, 61, 57, 54, 50, 46, 43, 38, 36, 31, 25,
        22, 18, 13, 9, 4, 0]


def _cosv(i):
    i %= 128
    if i <= 32:
        return _C32[i]
    if i <= 64:
        return -_C32[64 - i]
    if i <= 96:
        return -_C32[i - 64]
    return _C32[128 - i]


def dct_matrix(n):
    m = []
    for k in range(n):
        if k == 0:
            m.append([64] * n)
        else:
            m.append([_cosv((2 * j + 1) * k * (32 // n)) for j in range(n)])
    return m


DCT = {n: dct_matrix(n) for n in (4, 8, 16, 32)}


def scan_orders(blk):
    """scan position -> (x, y) for diagonal (0), horizontal (1), vertical (2) of a blk x blk array"""
    diag = []
    x = y = 0
    stop = False
    while not stop:
        while y >= 0:
            if x < blk and y < blk:
                diag.append((x, y))
            y -= 1
            x += 1
        y = x
        x = 0
        if len(diag) >= blk * blk:
            stop = True
    hor = [(x, y) for y in range(blk) for x in range(blk)]
    ver = [(x, y) for x in range(blk) for y in range(blk)]
    return [diag, hor, ver]


SCAN4 = scan_orders(4)
SCAN = {n: scan_orders(n) for n in (2, 4, 8)}


def clip3(lo, hi, v):
    return lo if v < lo else (hi if v > hi else v)


# ================================================================== parameter sets
class SPS:
    pass


class PPS:
    pass


def profile_tier_level(br, max_sub_layers_minus1):
    br.u(2); br.u(1); br.u(5)            # profile space, tier, profile idc
    br.u(32)                            # compatibility flags
    br.u(4); br.u(43); br.u(1)          # progressive.. flags, reserved, inbld
    br.u(8)                             # level idc
    sub_profile = []
    sub_level = []
    for _ in range(max_sub_layers_minus1):
        sub_profile.append(br.u(1))
        sub_level.append(br.u(1))
    if max_sub_layers_minus1 > 0:
        for _ in range(max_sub_layers_minus1, 8):
            br.u(2)
    for i in range(max_sub_layers_minus1):
        if sub_profile[i]:
            br.u(2); br.u(1); br.u(5); br.u(32); br.u(4); br.u(43); br.u(1)
        if sub_level[i]:
            br.u(8)


def scaling_list_data(br):
    sl = {}
    for size_id in range(4):
        step = 3 if size_id == 3 else 1
        for matrix_id in range(0, 6, step):
            pred_mode = br.u(1)
            n = min(64, 1 << (4 + (size_id << 1)))
            if not pred_mode:
                delta = br.ue()
                if delta == 0:
                    sl[(size_id, matrix_id)] = default_list(size_id, matrix_id)
                    if size_id > 1:
                        sl[(size_id, matrix_id, "dc")] = 16
                else:
                    ref = matrix_id - delta * (3 if size_id == 3 else 1)
                    sl[(size_id, matrix_id)] = list(sl[(size_id, ref)])
                    if size_id > 1:
                        sl[(size_id, matrix_id, "dc")] = sl[(size_id, ref, "dc")]
            else:
                nxt = 8
                if size_id > 1:
                    dc = br.se() + 8
                    nxt = dc
                    sl[(size_id, matrix_id, "dc")] = dc
                lst = []
                for i in range(n):
                    d = br.se()
                    nxt = (nxt + d + 256) % 256
                    lst.append(nxt)
                sl[(size_id, matrix_id)] = lst
    # 32x32 chroma (RExt 4:4:4): copied from 16x16 per the chroma 444 rule
    for m in (1, 2, 4, 5):
        if (3, m) not in sl:
            sl[(3, m)] = list(sl[(2, m)])
            sl[(3, m, "dc")] = sl[(2, m, "dc")]
    return sl


def default_list(size_id, matrix_id):
    if size_id == 0:
        return [16] * 16
    return list(DEFAULT_SL_INTRA8 if matrix_id < 3 else DEFAULT_SL_INTER8)


def default_scaling():
    sl = {}
    for size_id in range(4):
        for m in range(6):
            sl[(size_id, m)] = default_list(size_id, m)
            if size_id > 1:
                sl[(size_id, m, "dc")] = 16
    return sl


def scaling_factor(sl, log2, matrix_id):
    """ScalingFactor[sizeId][matrixId][x][y] as a dict (x, y) -> m"""
    n = 1 << log2
    size_id = log2 - 2
    lst = sl[(size_id, matrix_id)]
    f = {}
    if size_id == 0:
        for i, (x, y) in enumerate(SCAN4[0]):
            f[(x, y)] = lst[i]
        return f
    sc8 = SCAN[8][0]
    ratio = n // 8
    for i, (x, y) in enumerate(sc8):
        for dy in range(ratio):
            for dx in range(ratio):
                f[(x * ratio + dx, y * ratio + dy)] = lst[i]
    if size_id > 1:
        f[(0, 0)] = sl[(size_id, matrix_id, "dc")]
    return f


def parse_sps(rbsp):
    br = BitReader(rbsp)
    s = SPS()
    br.u(4)
    s.max_sub_layers_minus1 = br.u(3)
    br.u(1)
    profile_tier_level(br, s.max_sub_layers_minus1)
    s.id = br.ue()
    s.chroma_format_idc = br.ue()
    s.separate_colour_plane = br.u(1) if s.chroma_format_idc == 3 else 0
    s.width = br.ue()
    s.height = br.ue()
    s.conf = (0, 0, 0, 0)
    if br.u(1):
        s.conf = (br.ue(), br.ue(), br.ue(), br.ue())
    s.bit_depth_y = br.ue() + 8
    s.bit_depth_c = br.ue() + 8
    s.log2_max_poc = br.ue() + 4
    sub_ordering = br.u(1)
    for i in range(0 if sub_ordering else s.max_sub_layers_minus1, s.max_sub_layers_minus1 + 1):
        br.ue(); br.ue(); br.ue()
    s.log2_min_cb = br.ue() + 3
    s.log2_ctb = s.log2_min_cb + br.ue()
    s.log2_min_tb = br.ue() + 2
    s.log2_max_tb = s.log2_min_tb + br.ue()
    s.max_th_depth_inter = br.ue()
    s.max_th_depth_intra = br.ue()
    s.scaling_list_enabled = br.u(1)
    s.scaling = None
    if s.scaling_list_enabled:
        if br.u(1):
            s.scaling = scaling_list_data(br)
        else:
            s.scaling = default_scaling()
    s.amp = br.u(1)
    s.sao = br.u(1)
    s.pcm = br.u(1)
    if s.pcm:
        s.pcm_bd_y = br.u(4) + 1
        s.pcm_bd_c = br.u(4) + 1
        s.log2_min_pcm = br.ue() + 3
        s.log2_max_pcm = s.log2_min_pcm + br.ue()
        s.pcm_loop_filter_disabled = br.u(1)
    s.num_st_rps = br.ue()
    s.st_rps = []
    for i in range(s.num_st_rps):
        s.st_rps.append(st_ref_pic_set(br, i, s.num_st_rps, s.st_rps))
    s.long_term = br.u(1)
    if s.long_term:
        n = br.ue()
        for _ in range(n):
            br.u(s.log2_max_poc)
            br.u(1)
    s.temporal_mvp = br.u(1)
    s.strong_intra_smoothing = br.u(1)
    vui = br.u(1)
    if vui:
        parse_vui(br, s)
    # extensions
    s.implicit_rdpcm = s.extended_precision = s.intra_smoothing_disabled = 0
    s.high_precision_offsets = s.persistent_rice = s.cabac_bypass_alignment = 0
    s.ts_rotation = s.ts_context = s.explicit_rdpcm = 0
    if br.u(1):
        range_ext = br.u(1)
        br.u(1); br.u(1); br.u(1)
        br.u(4)
        if range_ext:
            s.ts_rotation = br.u(1)
            s.ts_context = br.u(1)
            s.implicit_rdpcm = br.u(1)
            s.explicit_rdpcm = br.u(1)
            s.extended_precision = br.u(1)
            s.intra_smoothing_disabled = br.u(1)
            s.high_precision_offsets = br.u(1)
            s.persistent_rice = br.u(1)
            s.cabac_bypass_alignment = br.u(1)
    s.chroma_array_type = 0 if s.separate_colour_plane else s.chroma_format_idc
    s.sub_w = 2 if s.chroma_format_idc in (1, 2) else 1
    s.sub_h = 2 if s.chroma_format_idc == 1 else 1
    s.ctb_size = 1 << s.log2_ctb
    s.pic_w_ctb = (s.width + s.ctb_size - 1) >> s.log2_ctb
    s.pic_h_ctb = (s.height + s.ctb_size - 1) >> s.log2_ctb
    return s


def parse_vui(br, s):
    if br.u(1):                     # aspect ratio
        if br.u(8) == 255:
            br.u(16); br.u(16)
    if br.u(1):                     # overscan
        br.u(1)
    s.vui_colour = None
    if br.u(1):                     # video signal type
        br.u(3)
        s.vui_full_range = br.u(1)
        if br.u(1):
            s.vui_colour = (br.u(8), br.u(8), br.u(8))
    if br.u(1):                     # chroma loc
        br.ue(); br.ue()
    br.u(1); br.u(1); br.u(1)       # neutral chroma, field seq, frame field info
    if br.u(1):                     # default display window
        br.ue(); br.ue(); br.ue(); br.ue()
    if br.u(1):                     # timing
        br.u(32); br.u(32)
        if br.u(1):
            br.ue()
        if br.u(1):
            hrd_parameters(br, 1, s.max_sub_layers_minus1)
    if br.u(1):                     # bitstream restriction
        br.u(1); br.u(1); br.u(1)
        br.ue(); br.ue(); br.ue(); br.ue(); br.ue()


def hrd_parameters(br, common, max_sub):
    nal = vcl = sub_pic = 0
    if common:
        nal = br.u(1)
        vcl = br.u(1)
        if nal or vcl:
            sub_pic = br.u(1)
            if sub_pic:
                br.u(8); br.u(5); br.u(1); br.u(5)
            br.u(4); br.u(4)
            if sub_pic:
                br.u(4)
            br.u(5); br.u(5); br.u(5)
    for i in range(max_sub + 1):
        fixed_general = br.u(1)
        fixed_within = 1 if fixed_general else br.u(1)
        low_delay = 0
        if fixed_within:
            br.ue()
        else:
            low_delay = br.u(1)
        cpb_cnt = 1
        if not low_delay:
            cpb_cnt = br.ue() + 1
        for _ in range((1 if nal else 0) + (1 if vcl else 0)):
            for _ in range(cpb_cnt):
                br.ue(); br.ue()
                if sub_pic:
                    br.ue(); br.ue()
                br.u(1)


def st_ref_pic_set(br, idx, num, sets):
    inter = br.u(1) if idx != 0 else 0
    if inter:
        delta_idx = br.ue() + 1 if idx == num else 1
        br.u(1)
        br.ue()
        ref = sets[idx - delta_idx]
        n = 0
        for _ in range(ref + 1):
            used = br.u(1)
            if not used:
                use_delta = br.u(1)
            else:
                use_delta = 1
            if used or use_delta:
                n += 1
        return n
    neg = br.ue()
    pos = br.ue()
    for _ in range(neg):
        br.ue(); br.u(1)
    for _ in range(pos):
        br.ue(); br.u(1)
    return neg + pos


def parse_pps(rbsp, spss):
    br = BitReader(rbsp)
    p = PPS()
    p.id = br.ue()
    p.sps_id = br.ue()
    p.dependent_slices = br.u(1)
    p.output_flag_present = br.u(1)
    p.num_extra_slice_header_bits = br.u(3)
    p.sign_data_hiding = br.u(1)
    p.cabac_init_present = br.u(1)
    br.ue(); br.ue()
    p.init_qp = 26 + br.se()
    p.constrained_intra = br.u(1)
    p.transform_skip = br.u(1)
    p.cu_qp_delta = br.u(1)
    p.diff_cu_qp_delta_depth = br.ue() if p.cu_qp_delta else 0
    p.cb_qp_offset = br.se()
    p.cr_qp_offset = br.se()
    p.slice_chroma_qp_offsets_present = br.u(1)
    br.u(1); br.u(1)                    # weighted pred / bipred
    p.transquant_bypass = br.u(1)
    p.tiles = br.u(1)
    p.entropy_sync = br.u(1)
    p.col_widths = p.row_heights = None
    p.loop_filter_across_tiles = 1
    if p.tiles:
        p.num_cols = br.ue() + 1
        p.num_rows = br.ue() + 1
        p.uniform = br.u(1)
        if not p.uniform:
            p.col_widths = [br.ue() + 1 for _ in range(p.num_cols - 1)]
            p.row_heights = [br.ue() + 1 for _ in range(p.num_rows - 1)]
        p.loop_filter_across_tiles = br.u(1)
    else:
        p.num_cols = p.num_rows = 1
        p.uniform = 1
    p.loop_filter_across_slices = br.u(1)
    p.deblocking_override_enabled = 0
    p.deblocking_disabled = 0
    p.beta_offset = p.tc_offset = 0
    if br.u(1):                         # deblocking filter control
        p.deblocking_override_enabled = br.u(1)
        p.deblocking_disabled = br.u(1)
        if not p.deblocking_disabled:
            p.beta_offset = br.se() * 2
            p.tc_offset = br.se() * 2
    p.scaling = None
    if br.u(1):
        p.scaling = scaling_list_data(br)
    p.lists_modification_present = br.u(1)
    p.log2_parallel_merge = br.ue() + 2
    p.slice_header_extension = br.u(1)
    p.log2_max_ts = 2
    p.cross_component = 0
    p.chroma_qp_offset_list = 0
    p.log2_sao_offset_scale_y = p.log2_sao_offset_scale_c = 0
    if br.u(1):
        range_ext = br.u(1)
        br.u(1); br.u(1); br.u(1); br.u(4)
        if range_ext:
            if p.transform_skip:
                p.log2_max_ts = br.ue() + 2
            p.cross_component = br.u(1)
            p.chroma_qp_offset_list = br.u(1)
            if p.chroma_qp_offset_list:
                p.diff_cu_chroma_qp_offset_depth = br.ue()
                n = br.ue() + 1
                p.cb_qp_offset_list = []
                p.cr_qp_offset_list = []
                for _ in range(n):
                    p.cb_qp_offset_list.append(br.se())
                    p.cr_qp_offset_list.append(br.se())
            p.log2_sao_offset_scale_y = br.ue()
            p.log2_sao_offset_scale_c = br.ue()
    return p


def setup_tiles(p, s):
    W, H = s.pic_w_ctb, s.pic_h_ctb
    if p.uniform:
        cw = [((i + 1) * W) // p.num_cols - (i * W) // p.num_cols for i in range(p.num_cols)]
        rh = [((i + 1) * H) // p.num_rows - (i * H) // p.num_rows for i in range(p.num_rows)]
    else:
        cw = p.col_widths + [W - sum(p.col_widths)]
        rh = p.row_heights + [H - sum(p.row_heights)]
    colbd = [0]
    for v in cw:
        colbd.append(colbd[-1] + v)
    rowbd = [0]
    for v in rh:
        rowbd.append(rowbd[-1] + v)
    rs2ts = [0] * (W * H)
    tile_id = [0] * (W * H)
    for rs in range(W * H):
        tbx, tby = rs % W, rs // W
        tx = max(i for i in range(p.num_cols) if tbx >= colbd[i])
        ty = max(j for j in range(p.num_rows) if tby >= rowbd[j])
        v = 0
        for i in range(tx):
            v += rh[ty] * cw[i]
        for j in range(ty):
            v += W * rh[j]
        v += (tby - rowbd[ty]) * cw[tx] + tbx - colbd[tx]
        rs2ts[rs] = v
    ts2rs = [0] * (W * H)
    for rs, ts in enumerate(rs2ts):
        ts2rs[ts] = rs
    tid = 0
    for j in range(p.num_rows):
        for i in range(p.num_cols):
            for y in range(rowbd[j], rowbd[j + 1]):
                for x in range(colbd[i], colbd[i + 1]):
                    tile_id[rs2ts[y * W + x]] = tid
            tid += 1
    p.colbd, p.rowbd, p.rs2ts, p.ts2rs, p.tile_id = colbd, rowbd, rs2ts, ts2rs, tile_id


# ================================================================== CABAC
class Cabac:
    def __init__(self, data, pos):
        self.d = data
        self.pos = pos            # byte position
        self.init_decoder()

    def init_decoder(self):
        self.range = 510
        self.bits_needed = -8
        self.value = 0
        self.value = (self.byte() << 8) | self.byte()
        self.value = (self.value << 1) & 0xFFFF
        # use the simple 9-bit formulation instead
        self.pos -= 2
        self.offset = 0
        self.bitpos = self.pos * 8
        self.offset = self.rbits(9)

    def byte(self):
        b = self.d[self.pos] if self.pos < len(self.d) else 0
        self.pos += 1
        return b

    def rbit(self):
        p = self.bitpos
        self.bitpos += 1
        if (p >> 3) >= len(self.d):
            return 0
        return (self.d[p >> 3] >> (7 - (p & 7))) & 1

    def rbits(self, n):
        v = 0
        for _ in range(n):
            v = (v << 1) | self.rbit()
        return v

    def decision(self, ctx):
        st, mps = ctx
        lps = LPS_TAB[st][(self.range >> 6) & 3]
        self.range -= lps
        if self.offset >= self.range:
            b = 1 - mps
            self.offset -= self.range
            self.range = lps
            if st == 0:
                mps = 1 - mps
            st = TRANS_LPS[st]
        else:
            b = mps
            st = TRANS_MPS[st]
        ctx[0], ctx[1] = st, mps
        while self.range < 256:
            self.range <<= 1
            self.offset = (self.offset << 1) | self.rbit()
        return b

    def bypass(self):
        self.offset = (self.offset << 1) | self.rbit()
        if self.offset >= self.range:
            self.offset -= self.range
            return 1
        return 0

    def bypass_bits(self, n):
        v = 0
        for _ in range(n):
            v = (v << 1) | self.bypass()
        return v

    def terminate(self):
        self.range -= 2
        if self.offset >= self.range:
            return 1
        while self.range < 256:
            self.range <<= 1
            self.offset = (self.offset << 1) | self.rbit()
        return 0

    def byte_pos_after_terminate(self):
        """byte position of the data after a terminating bin equal to 1: the decoder has read all
        R + 9 bits of the arithmetic codeword, whose last bit (1) is the rbsp_stop_one_bit /
        alignment_bit_equal_to_one; zero bits follow up to the byte boundary"""
        return (self.bitpos + 7) >> 3

    def reinit_at(self, bytepos):
        self.bitpos = bytepos * 8
        self.range = 510
        self.offset = self.rbits(9)


def init_ctx(name, qp):
    out = []
    for iv in INIT[name]:
        slope = iv >> 4
        off = iv & 15
        m = slope * 5 - 45
        n = (off << 3) - 16
        pre = clip3(1, 126, ((m * clip3(0, 51, qp)) >> 4) + n)
        mps = 1 if pre > 63 else 0
        st = (pre - 64) if mps else (63 - pre)
        out.append([st, mps])
    return out


def init_contexts(qp):
    return {k: init_ctx(k, qp) for k in INIT}


def copy_contexts(c):
    return {k: [list(x) for x in v] for k, v in c.items()}


# ================================================================== picture state
class Picture:
    pass


class Decoder:
    def __init__(self):
        self.vps = {}
        self.sps = {}
        self.pps = {}
        self.pic = None
        self.trace = None

    # -------------------------------------------------------------- NAL level
    def decode(self, nals):
        for nal in nals:
            if len(nal) < 2:
                continue
            t = (nal[0] >> 1) & 63
            rbsp = unescape(nal[2:])
            if t == 33:
                s = parse_sps(rbsp)
                self.sps[s.id] = s
            elif t == 34:
                p = parse_pps(rbsp, self.sps)
                self.pps[p.id] = p
            elif t <= 21:
                self.slice_segment(t, rbsp)
        return self.finish()

    # -------------------------------------------------------------- slice header
    def slice_segment(self, nal_type, rbsp):
        br = BitReader(rbsp)
        first = br.u(1)
        if 16 <= nal_type <= 23:
            br.u(1)
        pps = self.pps[br.ue()]
        sps = self.sps[pps.sps_id]
        if first:
            self.new_picture(sps, pps)
        sh = self.sh if not first else None
        dependent = 0
        addr = 0
        if not first:
            if pps.dependent_slices:
                dependent = br.u(1)
            nbits = (sps.pic_w_ctb * sps.pic_h_ctb - 1).bit_length()
            addr = br.u(nbits)
        if not dependent:
            sh = type("SH", (), {})()
            br.u(pps.num_extra_slice_header_bits)
            sh.slice_type = br.ue()
            if sh.slice_type != 2:
                raise NotImplementedError("only I slices")
            if pps.output_flag_present:
                br.u(1)
            if sps.separate_colour_plane:
                br.u(2)
            if nal_type not in (19, 20):        # not IDR
                br.u(sps.log2_max_poc)
                st_flag = br.u(1)
                if not st_flag:
                    st_ref_pic_set(br, sps.num_st_rps, sps.num_st_rps, sps.st_rps)
                elif sps.num_st_rps > 1:
                    br.u((sps.num_st_rps - 1).bit_length())
                if sps.long_term:
                    raise NotImplementedError("long-term refs in an I picture header")
                if sps.temporal_mvp:
                    br.u(1)
            sh.sao_luma = sh.sao_chroma = 0
            if sps.sao:
                sh.sao_luma = br.u(1)
                if sps.chroma_array_type != 0:
                    sh.sao_chroma = br.u(1)
            sh.qp_delta = br.se()
            sh.cb_qp_offset = sh.cr_qp_offset = 0
            if pps.slice_chroma_qp_offsets_present:
                sh.cb_qp_offset = br.se()
                sh.cr_qp_offset = br.se()
            sh.cu_chroma_qp_offset_enabled = 0
            if pps.chroma_qp_offset_list:
                sh.cu_chroma_qp_offset_enabled = br.u(1)
            override = 0
            if pps.deblocking_override_enabled:
                override = br.u(1)
            sh.deblocking_disabled = pps.deblocking_disabled
            sh.beta_offset, sh.tc_offset = pps.beta_offset, pps.tc_offset
            if override:
                sh.deblocking_disabled = br.u(1)
                if not sh.deblocking_disabled:
                    sh.beta_offset = br.se() * 2
                    sh.tc_offset = br.se() * 2
            sh.loop_filter_across_slices = pps.loop_filter_across_slices
            if pps.loop_filter_across_slices and (sh.sao_luma or sh.sao_chroma or not sh.deblocking_disabled):
                sh.loop_filter_across_slices = br.u(1)
            sh.qp = pps.init_qp + sh.qp_delta
        if pps.tiles or pps.entropy_sync:
            n = br.ue()
            if n > 0:
                off_bits = br.ue() + 1
                for _ in range(n):
                    br.u(off_bits)
        if pps.slice_header_extension:
            n = br.ue()
            br.u(8 * n)
        # byte_alignment()
        br.u(1)
        while not br.byte_aligned():
            br.u(1)
        self.sh = sh
        if not dependent:
            self.slice_addr = addr          # SliceAddrRs of independent slice
        self.decode_slice_data(rbsp, br.pos >> 3, addr, dependent)

    # -------------------------------------------------------------- picture setup
    def new_picture(self, sps, pps):
        s = sps
        self.s, self.p = sps, pps
        setup_tiles(pps, sps)
        pic = Picture()
        pic.w, pic.h = s.width, s.height
        ncomp = 1 if s.chroma_array_type == 0 else 3
        pic.planes = []
        for c in range(ncomp):
            w = s.width if c == 0 else s.width // s.sub_w
            h = s.height if c == 0 else s.height // s.sub_h
            pic.planes.append([[0] * w for _ in range(h)])
        self.pic = pic
        mw, mh = (s.width + 3) >> 2, (s.height + 3) >> 2      # 4x4 grid
        self.mw, self.mh = mw, mh
        self.dec = [[-1] * mw for _ in range(mh)]            # slice address (decoded) per 4x4, -1 = not yet
        self.tid = [[-1] * mw for _ in range(mh)]
        self.ctdepth = [[0] * mw for _ in range(mh)]
        self.ipm = [[1] * mw for _ in range(mh)]             # luma intra mode per 4x4
        self.qpy = [[0] * mw for _ in range(mh)]
        self.bypass = [[0] * mw for _ in range(mh)]          # pcm (filter-disabled) / transquant bypass
        self.pcm_map = [[0] * mw for _ in range(mh)]
        self.edge_v = [[0] * mw for _ in range(mh)]          # TU / PU edge at the left of the 4x4 (bS 2)
        self.edge_h = [[0] * mw for _ in range(mh)]
        self.slice_of_ctb = [0] * (s.pic_w_ctb * s.pic_h_ctb)
        self.deblock_off_ctb = [0] * (s.pic_w_ctb * s.pic_h_ctb)
        self.lf_across_ctb = [1] * (s.pic_w_ctb * s.pic_h_ctb)
        self.betatc_ctb = [(0, 0)] * (s.pic_w_ctb * s.pic_h_ctb)
        self.chroma_off_ctb = [(0, 0)] * (s.pic_w_ctb * s.pic_h_ctb)
        self.sao_params = [None] * (s.pic_w_ctb * s.pic_h_ctb)
        self.wpp_ctx = None

    # -------------------------------------------------------------- slice data
    def decode_slice_data(self, data, bytepos, addr, dependent):
        s, p, sh = self.s, self.p, self.sh
        self.cab = Cabac(data, bytepos)
        ctb_ts = p.rs2ts[addr]
        if dependent and self.saved_ctx is not None:
            self.ctx = copy_contexts(self.saved_ctx)
        else:
            self.ctx = init_contexts(sh.qp)
        self.stat_coeff = [0] * 4
        if not dependent:
            self.qpy_prev = sh.qp
        else:
            self.qpy_prev = self.qpy_prev_end
        first_ctb_in_slice = True
        while True:
            rs = p.ts2rs[ctb_ts]
            rx, ry = rs % s.pic_w_ctb, rs // s.pic_w_ctb
            # new tile / WPP row: context handling
            if not first_ctb_in_slice:
                if p.tiles and p.tile_id[ctb_ts] != p.tile_id[ctb_ts - 1]:
                    self.ctx = init_contexts(sh.qp)
                    self.qpy_prev = sh.qp
                elif p.entropy_sync and rx == p.colbd[self.tile_col(rx)]:
                    self.wpp_sync(rx, ry)
            elif p.entropy_sync and rx == p.colbd[self.tile_col(rx)] and not (dependent and self.saved_ctx is not None and False):
                # first CTB of a slice at a row start: WPP sync when the CTB above-right is available
                self.wpp_sync(rx, ry, slice_start=True)
            if p.entropy_sync and rx == p.colbd[self.tile_col(rx)]:
                self.qpy_prev = sh.qp
            self.slice_of_ctb[rs] = self.slice_addr
            self.deblock_off_ctb[rs] = sh.deblocking_disabled
            self.lf_across_ctb[rs] = sh.loop_filter_across_slices
            self.betatc_ctb[rs] = (sh.beta_offset, sh.tc_offset)
            self.chroma_off_ctb[rs] = (sh.cb_qp_offset, sh.cr_qp_offset)
            self.coding_tree_unit(rx, ry, ctb_ts)
            first_ctb_in_slice = False
            # WPP: store contexts after the 2nd CTB of a row
            if p.entropy_sync and rx == p.colbd[self.tile_col(rx)] + 1 - (1 if p.colbd[self.tile_col(rx) + 1] - p.colbd[self.tile_col(rx)] == 1 else 0):
                pass
            if p.entropy_sync:
                tc = self.tile_col(rx)
                if rx == min(p.colbd[tc] + 1, p.colbd[tc + 1] - 1) and (p.colbd[tc + 1] - p.colbd[tc] >= 2):
                    self.wpp_ctx = (ry, tc, copy_contexts(self.ctx), list(self.stat_coeff))
                elif p.colbd[tc + 1] - p.colbd[tc] == 1:
                    self.wpp_ctx = (ry, tc, copy_contexts(self.ctx), list(self.stat_coeff))
            end = self.cab.terminate()
            ctb_ts += 1
            if end:
                self.saved_ctx = copy_contexts(self.ctx)
                self.qpy_prev_end = self.qpy_prev
                break
            if ctb_ts >= len(p.ts2rs):
                raise ValueError("slice runs past the picture")
            nrs = p.ts2rs[ctb_ts]
            nrx = nrs % s.pic_w_ctb
            if (p.tiles and p.tile_id[ctb_ts] != p.tile_id[ctb_ts - 1]) or \
               (p.entropy_sync and nrx == p.colbd[self.tile_col(nrx)]):
                if not self.cab.terminate():
                    raise ValueError("end_of_subset_one_bit")
                self.cab.reinit_at(self.cab.byte_pos_after_terminate())

    def tile_col(self, rx):
        p = self.p
        return max(i for i in range(p.num_cols) if rx >= p.colbd[i])

    def wpp_sync(self, rx, ry, slice_start=False):
        s, p, sh = self.s, self.p, self.sh
        tc = self.tile_col(rx)
        # spatial neighbour (x + CtbSize, y - CtbSize) available: same slice?, same tile, decoded
        avail = False
        if ry > 0 and self.wpp_ctx is not None:
            wy, wtc, wctx, wst = self.wpp_ctx
            xr = min(rx + 1, p.colbd[tc + 1] - 1)
            rs_tr = (ry - 1) * s.pic_w_ctb + xr
            if wy == ry - 1 and wtc == tc and p.tile_id[p.rs2ts[rs_tr]] == p.tile_id[p.rs2ts[ry * s.pic_w_ctb + rx]]                     and self.slice_of_ctb[rs_tr] == self.slice_addr:
                avail = self.ctb_decoded(rs_tr)
        if avail:
            self.ctx = copy_contexts(self.wpp_ctx[2])
            self.stat_coeff = list(self.wpp_ctx[3])
        else:
            if not slice_start:
                self.ctx = init_contexts(sh.qp)
                self.stat_coeff = [0] * 4

    def ctb_decoded(self, rs):
        s = self.s
        x = (rs % s.pic_w_ctb) << s.log2_ctb
        y = (rs // s.pic_w_ctb) << s.log2_ctb
        return self.dec[y >> 2][x >> 2] >= 0

    # -------------------------------------------------------------- availability
    def available(self, xc, yc, xn, yn):
        """6.4.1 z-scan availability of (xn, yn) for the block at (xc, yc): decoded already, in
        the picture, same slice and same tile"""
        s = self.s
        if xn < 0 or yn < 0 or xn >= s.width or yn >= s.height:
            return False
        a = self.dec[yn >> 2][xn >> 2]
        if a < 0:
            return False
        if a != self.slice_addr:
            return False
        if self.tid[yn >> 2][xn >> 2] != self.cur_tile:
            return False
        return True

    # -------------------------------------------------------------- CTU
    def coding_tree_unit(self, rx, ry, ctb_ts):
        s, p, sh = self.s, self.p, self.sh
        self.cur_tile = p.tile_id[ctb_ts]
        x0, y0 = rx << s.log2_ctb, ry << s.log2_ctb
        rs = ry * s.pic_w_ctb + rx
        if sh.sao_luma or sh.sao_chroma:
            self.sao_syntax(rx, ry, rs)
        else:
            self.sao_params[rs] = None
        self.coding_quadtree(x0, y0, s.log2_ctb, 0)

    def sao_syntax(self, rx, ry, rs):
        s, p, sh = self.s, self.p, self.sh
        cab = self.cab
        if rx > 0:
            lrs = rs - 1
            left_ok = self.slice_of_ctb[lrs] == self.slice_addr and p.tile_id[p.rs2ts[lrs]] == p.tile_id[p.rs2ts[rs]] \
                and self.ctb_decoded(lrs)
            if left_ok and cab.decision(self.ctx["sao_merge"][0]):
                self.sao_params[rs] = self.sao_params[lrs]
                return
        if ry > 0:
            urs = rs - s.pic_w_ctb
            up_ok = self.slice_of_ctb[urs] == self.slice_addr and p.tile_id[p.rs2ts[urs]] == p.tile_id[p.rs2ts[rs]] \
                and self.ctb_decoded(urs)
            if up_ok and cab.decision(self.ctx["sao_merge"][0]):
                self.sao_params[rs] = self.sao_params[urs]
                return
        params = []
        ncomp = 3 if s.chroma_array_type != 0 else 1
        typ = eo = 0
        for c in range(ncomp):
            on = sh.sao_luma if c == 0 else sh.sao_chroma
            if not on:
                params.append(None)
                continue
            if c == 0 or c == 1:
                if cab.decision(self.ctx["sao_type"][0]) == 0:
                    typ = 0
                else:
                    typ = 2 if cab.bypass() else 1
            if typ == 0:
                params.append(None)
                continue
            bd = s.bit_depth_y if c == 0 else s.bit_depth_c
            cmax = (1 << (min(bd, 10) - 5)) - 1
            offs = []
            for _ in range(4):
                v = 0
                while v < cmax and cab.bypass():
                    v += 1
                offs.append(v)
            scale = self.p.log2_sao_offset_scale_y if c == 0 else self.p.log2_sao_offset_scale_c
            if typ == 1:
                for i in range(4):
                    if offs[i] and cab.bypass():
                        offs[i] = -offs[i]
                band = cab.bypass_bits(5)
                params.append(("band", band, [o << scale for o in offs]))
            else:
                if c == 0:
                    eo = cab.bypass_bits(2)
                if c == 1:
                    eo = cab.bypass_bits(2)
                vals = [offs[0], offs[1], -offs[2], -offs[3]]
                params.append(("edge", eo, [v << scale for v in vals]))
        self.sao_params[rs] = params

    # -------------------------------------------------------------- coding quadtree
    def coding_quadtree(self, x0, y0, log2, depth):
        s, p = self.s, self.p
        size = 1 << log2
        if x0 + size <= s.width and y0 + size <= s.height and log2 > s.log2_min_cb:
            inc = 0
            if self.available(x0, y0, x0 - 1, y0) and self.ctdepth[y0 >> 2][(x0 - 1) >> 2] > depth:
                inc += 1
            if self.available(x0, y0, x0, y0 - 1) and self.ctdepth[(y0 - 1) >> 2][x0 >> 2] > depth:
                inc += 1
            split = self.cab.decision(self.ctx["split_cu"][inc])
        else:
            split = 1 if log2 > s.log2_min_cb else 0
        if p.cu_qp_delta and log2 >= s.log2_ctb - p.diff_cu_qp_delta_depth:
            self.is_cu_qp_delta_coded = 0
            self.cu_qp_delta_val = 0
            self.qg_x, self.qg_y = x0, y0
            self.new_qg = True
        if self.sh.cu_chroma_qp_offset_enabled and log2 >= s.log2_ctb - p.diff_cu_chroma_qp_offset_depth:
            self.is_cu_chroma_qp_offset_coded = 0
        if split:
            h = size >> 1
            for (dx, dy) in ((0, 0), (h, 0), (0, h), (h, h)):
                if x0 + dx < s.width and y0 + dy < s.height:
                    self.coding_quadtree(x0 + dx, y0 + dy, log2 - 1, depth + 1)
        else:
            self.coding_unit(x0, y0, log2, depth)

    # -------------------------------------------------------------- QP
    def qp_pred(self, xcb, ycb):
        s, p = self.s, self.p
        # quantization group of the CU
        mask = ~((1 << (s.log2_ctb - p.diff_cu_qp_delta_depth)) - 1)
        xqg, yqg = xcb & mask, ycb & mask
        if self.new_qg_pred.get((xqg, yqg)) is None:
            prev = self.qpy_prev
            ctb_mask = ~((1 << s.log2_ctb) - 1)

            def nb(xn, yn):
                if not self.available(xcb, ycb, xn, yn):
                    return prev
                if (xn & ctb_mask) != (xqg & ctb_mask) or (yn & ctb_mask) != (yqg & ctb_mask):
                    return prev
                return self.qpy[yn >> 2][xn >> 2]
            qa = nb(xqg - 1, yqg)
            qb = nb(xqg, yqg - 1)
            self.new_qg_pred[(xqg, yqg)] = (qa + qb + 1) >> 1
        return self.new_qg_pred[(xqg, yqg)]

    # -------------------------------------------------------------- coding unit
    def coding_unit(self, x0, y0, log2, depth):
        s, p, sh = self.s, self.p, self.sh
        cab = self.cab
        size = 1 << log2
        if not hasattr(self, "new_qg_pred") or getattr(self, "new_qg", False):
            if not hasattr(self, "new_qg_pred"):
                self.new_qg_pred = {}
            if getattr(self, "new_qg", False):
                self.new_qg_pred = {}
                self.new_qg = False
        if not p.cu_qp_delta:
            self.cu_qp_delta_val = 0
            if not hasattr(self, "qg_x"):
                self.qg_x = self.qg_y = 0
        self.cu_x, self.cu_y, self.cu_log2 = x0, y0, log2
        bypass = cab.decision(self.ctx["tq_bypass"][0]) if p.transquant_bypass else 0
        part_nxn = 0
        if log2 == s.log2_min_cb:
            if log2 > s.log2_min_tb or True:
                part_nxn = 0 if cab.decision(self.ctx["part_mode"][0]) else 1
                if log2 == 3 and s.log2_min_tb > 2:
                    part_nxn = 0
        pcm = 0
        if not part_nxn and s.pcm and s.log2_min_pcm <= log2 <= s.log2_max_pcm:
            pcm = cab.terminate()
        # QP of the CU (predicted; a cu_qp_delta in a TU updates it)
        qp_pred = self.qp_pred(x0, y0) if p.cu_qp_delta else (self.qpy_prev if False else sh.qp)
        self.cu_qp_pred = qp_pred
        self.cu_qpy = self.calc_qpy(qp_pred, self.cu_qp_delta_val)
        self.cu_bypass = bypass
        self.cu_pcm = pcm
        nparts = 4 if part_nxn else 1
        modes = []
        if pcm:
            self.pcm_sample(x0, y0, log2)
            modes = [1]
            self.set_cu_info(x0, y0, size, depth, [1] * 4, part_nxn, filter_off=s.pcm_loop_filter_disabled)
        else:
            pb = size >> 1 if part_nxn else size
            flags = [cab.decision(self.ctx["prev_intra"][0]) for _ in range(nparts)]
            for i in range(nparts):
                xp = x0 + (i & 1) * pb
                yp = y0 + (i >> 1) * pb
                if flags[i]:
                    mpm_idx = 0
                    while mpm_idx < 2 and cab.bypass():
                        mpm_idx += 1
                    rem = None
                else:
                    mpm_idx = None
                    rem = cab.bypass_bits(5)
                cand = self.mpm_list(xp, yp)
                if mpm_idx is not None:
                    mode = cand[mpm_idx]
                else:
                    c = sorted(cand)
                    mode = rem
                    for v in c:
                        if mode >= v:
                            mode += 1
                modes.append(mode)
                for yy in range(yp, min(yp + pb, s.height), 4):
                    for xx in range(xp, min(xp + pb, s.width), 4):
                        self.ipm[yy >> 2][xx >> 2] = mode
            # chroma modes
            cmodes = []
            if s.chroma_array_type != 0:
                nc = 4 if (s.chroma_array_type == 3 and part_nxn) else 1
                for i in range(nc):
                    if cab.decision(self.ctx["chroma_mode"][0]) == 0:
                        icm = 4
                    else:
                        icm = cab.bypass_bits(2)
                    lm = modes[i]
                    if icm == 4:
                        cm = lm
                    else:
                        cm = [0, 26, 10, 1][icm]
                        if cm == lm:
                            cm = 34
                    if s.chroma_array_type == 2:
                        cm = MODE_422[cm]
                    cmodes.append(cm)
            self.cu_modes = modes
            self.cu_cmodes = cmodes
            self.cu_nxn = part_nxn
            self.set_cu_info(x0, y0, size, depth, modes, part_nxn, filter_off=bypass)
            max_depth = s.max_th_depth_intra + part_nxn
            self.transform_tree(x0, y0, x0, y0, log2, 0, 0, max_depth, part_nxn, [1, 1])
        # final QP of the CU
        for yy in range(y0, min(y0 + size, s.height), 4):
            for xx in range(x0, min(x0 + size, s.width), 4):
                self.qpy[yy >> 2][xx >> 2] = self.cu_qpy
        self.qpy_prev = self.cu_qpy

    def calc_qpy(self, pred, delta):
        s = self.s
        off = 6 * (s.bit_depth_y - 8)
        return ((pred + delta + 52 + 2 * off) % (52 + off)) - off

    def set_cu_info(self, x0, y0, size, depth, modes, nxn, filter_off):
        s = self.s
        for yy in range(y0, min(y0 + size, s.height), 4):
            for xx in range(x0, min(x0 + size, s.width), 4):
                self.ctdepth[yy >> 2][xx >> 2] = depth
                self.bypass[yy >> 2][xx >> 2] = filter_off
        # CU (and PU for NxN) edges
        for yy in range(y0, min(y0 + size, s.height), 4):
            self.edge_v[yy >> 2][x0 >> 2] = 1
        for xx in range(x0, min(x0 + size, s.width), 4):
            self.edge_h[y0 >> 2][xx >> 2] = 1
        if nxn:
            h = size >> 1
            if x0 + h < s.width:
                for yy in range(y0, min(y0 + size, s.height), 4):
                    self.edge_v[yy >> 2][(x0 + h) >> 2] = 1
            if y0 + h < s.height:
                for xx in range(x0, min(x0 + size, s.width), 4):
                    self.edge_h[(y0 + h) >> 2][xx >> 2] = 1

    def mark_decoded(self, x0, y0, w, h):
        s = self.s
        for yy in range(y0, min(y0 + h, s.height), 4):
            for xx in range(x0, min(x0 + w, s.width), 4):
                self.dec[yy >> 2][xx >> 2] = self.slice_addr
                self.tid[yy >> 2][xx >> 2] = self.cur_tile

    def mpm_list(self, xp, yp):
        s = self.s

        def cand(xn, yn, above):
            # z-scan availability: PUs of the current CU before this one count as decoded
            inside = self.cu_x <= xn < self.cu_x + (1 << self.cu_log2) and                 self.cu_y <= yn < self.cu_y + (1 << self.cu_log2)
            if not inside and not self.available(xp, yp, xn, yn):
                return 1
            if not inside and self.pcm_map[yn >> 2][xn >> 2]:
                return 1
            if above and yn < ((yp >> s.log2_ctb) << s.log2_ctb):
                return 1
            return self.ipm[yn >> 2][xn >> 2]
        a = cand(xp - 1, yp, False)
        b = cand(xp, yp - 1, True)
        if a == b:
            if a < 2:
                return [0, 1, 26]
            return [a, 2 + ((a + 29) % 32), 2 + ((a - 2 + 1) % 32)]
        if a != 0 and b != 0:
            c = 0
        elif a != 1 and b != 1:
            c = 1
        else:
            c = 26
        return [a, b, c]

    def pcm_sample(self, x0, y0, log2):
        s = self.s
        cab = self.cab
        # pcm_alignment_zero_bits, samples, then the arithmetic decoder restarts
        bp = cab.byte_pos_after_terminate()
        br = BitReader(cab.d)
        br.pos = bp * 8
        size = 1 << log2
        for y in range(size):
            for x in range(size):
                v = br.u(s.pcm_bd_y)
                self.put(0, x0 + x, y0 + y, v << (s.bit_depth_y - s.pcm_bd_y))
        if s.chroma_array_type != 0:
            cw, ch = size // s.sub_w, size // s.sub_h
            for c in (1, 2):
                for y in range(ch):
                    for x in range(cw):
                        v = br.u(s.pcm_bd_c)
                        self.put(c, x0 // s.sub_w + x, y0 // s.sub_h + y, v << (s.bit_depth_c - s.pcm_bd_c))
        cab.reinit_at(br.pos >> 3)
        self.mark_decoded(x0, y0, size, size)
        for yy in range(y0, min(y0 + size, s.height), 4):
            for xx in range(x0, min(x0 + size, s.width), 4):
                self.bypass[yy >> 2][xx >> 2] = 2 if s.pcm_loop_filter_disabled else 0
                self.pcm_map[yy >> 2][xx >> 2] = 1

    def put(self, c, x, y, v):
        pl = self.pic.planes[c]
        if 0 <= y < len(pl) and 0 <= x < len(pl[0]):
            pl[y][x] = v

    def get(self, c, x, y):
        return self.pic.planes[c][y][x]

    # -------------------------------------------------------------- transform tree
    def transform_tree(self, x0, y0, xb, yb, log2, depth, blk, max_depth, nxn, parent_cbf):
        s = self.s
        cab = self.cab
        if log2 <= s.log2_max_tb and log2 > s.log2_min_tb and depth < max_depth and not (nxn and depth == 0):
            split = cab.decision(self.ctx["split_tf"][5 - log2])
        else:
            split = 1 if (log2 > s.log2_max_tb or (nxn and depth == 0)) else 0
        cat = s.chroma_array_type
        cbf_cb = [0, 0]
        cbf_cr = [0, 0]
        if (log2 > 2 and cat != 0) or cat == 3:
            for comp, arr in ((0, cbf_cb), (1, cbf_cr)):
                if depth == 0 or parent_cbf[comp]:
                    arr[0] = cab.decision(self.ctx["cbf_chroma"][depth])
                    if cat == 2 and (not split or log2 == 3):
                        arr[1] = cab.decision(self.ctx["cbf_chroma"][depth])
        elif depth > 0 and log2 == 2 and cat != 0:
            # 4x4 luma TBs of 4:2:0 / 4:2:2: chroma flags inherited
            cbf_cb = list(self.parent_cbfs[0])
            cbf_cr = list(self.parent_cbfs[1])
        if split:
            h = 1 << (log2 - 1)
            saved = self.parent_cbfs if hasattr(self, "parent_cbfs") else None
            self.parent_cbfs = (cbf_cb, cbf_cr)
            pc = [1 if (cbf_cb[0] or cbf_cb[1]) else 0, 1 if (cbf_cr[0] or cbf_cr[1]) else 0]
            for i, (dx, dy) in enumerate(((0, 0), (h, 0), (0, h), (h, h))):
                self.parent_cbfs = (cbf_cb, cbf_cr)
                self.transform_tree(x0 + dx, y0 + dy, x0, y0, log2 - 1, depth + 1, i, max_depth, nxn, pc)
            self.parent_cbfs = saved
        else:
            cbf_luma = cab.decision(self.ctx["cbf_luma"][1 if depth == 0 else 0])
            self.transform_unit(x0, y0, xb, yb, log2, depth, blk, cbf_luma, cbf_cb, cbf_cr)

    # -------------------------------------------------------------- transform unit
    def transform_unit(self, x0, y0, xb, yb, log2, depth, blk, cbf_luma, cbf_cb, cbf_cr):
        s, p, sh = self.s, self.p, self.sh
        cab = self.cab
        cat = s.chroma_array_type
        size = 1 << log2
        cbf_chroma = any(cbf_cb) or any(cbf_cr)
        # TU edges for deblocking
        for yy in range(y0, min(y0 + size, s.height), 4):
            self.edge_v[yy >> 2][x0 >> 2] = 1
        for xx in range(x0, min(x0 + size, s.width), 4):
            self.edge_h[y0 >> 2][xx >> 2] = 1
        if cbf_luma or cbf_chroma:
            if p.cu_qp_delta and not self.is_cu_qp_delta_coded:
                v = 0
                while v < 5 and cab.decision(self.ctx["cu_qp_delta"][0 if v == 0 else 1]):
                    v += 1
                if v == 5:
                    k = 0
                    while cab.bypass():
                        k += 1
                    v += (1 << k) - 1 + cab.bypass_bits(k)
                if v and cab.bypass():
                    v = -v
                self.is_cu_qp_delta_coded = 1
                self.cu_qp_delta_val = v
                self.cu_qpy = self.calc_qpy(self.cu_qp_pred, v)
            if sh.cu_chroma_qp_offset_enabled and cbf_chroma and not self.cu_bypass and \
                    not self.is_cu_chroma_qp_offset_coded:
                raise NotImplementedError("cu_chroma_qp_offset")
        nxn = self.cu_nxn
        # luma
        mode = self.ipm[y0 >> 2][x0 >> 2]
        self.intra_pred(0, x0, y0, log2, mode)
        if cbf_luma:
            self.residual(0, x0, y0, log2, mode)
        self.mark_decoded(x0, y0, size, size)
        if cat == 0:
            return
        # chroma
        if log2 > 2 or cat == 3:
            log2c = log2 - (1 if cat != 3 else 0)
            xc, yc = x0 // s.sub_w, y0 // s.sub_h
            cidx_mode = self.chroma_mode_at(x0, y0)
            for comp, cbf in ((1, cbf_cb), (2, cbf_cr)):
                for t in range(2 if cat == 2 else 1):
                    yy = yc + (t << log2c)
                    self.intra_pred(comp, xc, yy, log2c, cidx_mode)
                    if cbf[t]:
                        self.residual(comp, xc, yy, log2c, cidx_mode)
        elif blk == 3:
            xc, yc = xb // s.sub_w, yb // s.sub_h
            cidx_mode = self.chroma_mode_at(xb, yb)
            for comp, cbf in ((1, cbf_cb), (2, cbf_cr)):
                for t in range(2 if cat == 2 else 1):
                    yy = yc + (t << 2)
                    self.intra_pred(comp, xc, yy, 2, cidx_mode)
                    if cbf[t]:
                        self.residual(comp, xc, yy, 2, cidx_mode)

    def chroma_mode_at(self, x0, y0):
        if len(self.cu_cmodes) == 4:
            h = 1 << (self.cu_log2 - 1)
            i = (1 if x0 >= self.cu_x + h else 0) + (2 if y0 >= self.cu_y + h else 0)
            return self.cu_cmodes[i]
        return self.cu_cmodes[0]

    # -------------------------------------------------------------- intra prediction (8.4.4.2)
    def intra_pred(self, c, x0, y0, log2, mode):
        s = self.s
        n = 1 << log2
        bd = s.bit_depth_y if c == 0 else s.bit_depth_c
        sw = 1 if c == 0 else s.sub_w
        shh = 1 if c == 0 else s.sub_h
        # luma position of the block for availability
        xl, yl = x0 * sw, y0 * shh
        # reference samples p[x][-1] for x = -1..2n-1 and p[-1][y] for y = 0..2n-1
        ref_top = [None] * (2 * n + 1)      # index x + 1
        ref_left = [None] * (2 * n)         # index y
        pl = self.pic.planes[c]
        for x in range(-1, 2 * n):
            xn, yn = x0 + x, y0 - 1
            if self.available(xl, yl, xn * sw, yn * shh):
                ref_top[x + 1] = pl[yn][xn]
        for y in range(2 * n):
            xn, yn = x0 - 1, y0 + y
            if self.available(xl, yl, xn * sw, yn * shh):
                ref_left[y] = pl[yn][xn]
        # substitution (8.4.4.2.2): order p[-1][2n-1] .. p[-1][-1] .. p[2n-1][-1]
        seq = [ref_left[y] for y in range(2 * n - 1, -1, -1)] + ref_top
        if all(v is None for v in seq):
            seq = [1 << (bd - 1)] * len(seq)
        else:
            if seq[0] is None:
                seq[0] = next(v for v in seq if v is not None)
            for i in range(1, len(seq)):
                if seq[i] is None:
                    seq[i] = seq[i - 1]
        left = [seq[2 * n - 1 - y] for y in range(2 * n)]      # p[-1][y]
        top = seq[2 * n:]                                        # p[x][-1], x = -1..2n-1 (index x + 1)
        # filtering
        if c == 0 or s.chroma_array_type == 3:
            filt = False
            if mode != 1 and n != 4:
                md = min(abs(mode - 26), abs(mode - 10))
                thr = {8: 7, 16: 1, 32: 0}[n]
                filt = md > thr
            if s.intra_smoothing_disabled:
                filt = False
            if filt:
                pc = top[0]
                if s.strong_intra_smoothing and c == 0 and n == 32 and \
                        abs(pc + top[2 * n] - 2 * top[n]) < (1 << (bd - 5)) and \
                        abs(pc + left[2 * n - 1] - 2 * left[n - 1]) < (1 << (bd - 5)):
                    nt = [pc] + [((63 - x) * pc + (x + 1) * top[2 * n] + 32) >> 6 for x in range(2 * n - 1)] + [top[2 * n]]
                    nl = [((63 - y) * pc + (y + 1) * left[2 * n - 1] + 32) >> 6 for y in range(2 * n - 1)] + [left[2 * n - 1]]
                    top, left = nt, nl
                else:
                    nt = [None] * (2 * n + 1)
                    nl = [None] * (2 * n)
                    nt[0] = (left[0] + 2 * pc + top[1] + 2) >> 2
                    for x in range(0, 2 * n - 1):
                        nt[x + 1] = (top[x] + 2 * top[x + 1] + top[x + 2] + 2) >> 2
                    nt[2 * n] = top[2 * n]
                    nl[0] = (pc + 2 * left[0] + left[1] + 2) >> 2
                    for y in range(1, 2 * n - 1):
                        nl[y] = (left[y - 1] + 2 * left[y] + left[y + 1] + 2) >> 2
                    nl[2 * n - 1] = left[2 * n - 1]
                    top, left = nt, nl
        # prediction
        pred = [[0] * n for _ in range(n)]
        maxv = (1 << bd) - 1
        if mode == 0:
            sh = log2 + 1
            for y in range(n):
                for x in range(n):
                    pred[y][x] = ((n - 1 - x) * left[y] + (x + 1) * top[n + 1] +
                                  (n - 1 - y) * top[x + 1] + (y + 1) * left[n] + n) >> sh
        elif mode == 1:
            dc = (sum(top[1:n + 1]) + sum(left[0:n]) + n) >> (log2 + 1)
            for y in range(n):
                for x in range(n):
                    pred[y][x] = dc
            if c == 0 and n < 32:
                pred[0][0] = (left[0] + 2 * dc + top[1] + 2) >> 2
                for x in range(1, n):
                    pred[0][x] = (top[x + 1] + 3 * dc + 2) >> 2
                for y in range(1, n):
                    pred[y][0] = (left[y] + 3 * dc + 2) >> 2
        else:
            ang = INTRA_ANGLE[mode]
            disable_bf = s.implicit_rdpcm and self.cu_bypass
            if mode >= 18:
                refm = {x: top[x + 1] for x in range(-1, 2 * n)}     # ref[x] = p[-1 + x][-1]
                ref = {}
                for x in range(0, n + 1):
                    ref[x] = top[x]                                   # p[-1 + x][-1]
                if ang < 0:
                    inv = INV_ANGLE[mode]
                    if (n * ang) >> 5 < -1:
                        for x in range((n * ang) >> 5, 0):
                            ref[x] = left[-1 + ((x * inv + 128) >> 8)]
                else:
                    for x in range(n + 1, 2 * n + 1):
                        ref[x] = top[x]
                for y in range(n):
                    idx = ((y + 1) * ang) >> 5
                    fact = ((y + 1) * ang) & 31
                    for x in range(n):
                        if fact:
                            pred[y][x] = ((32 - fact) * ref[x + idx + 1] + fact * ref[x + idx + 2] + 16) >> 5
                        else:
                            pred[y][x] = ref[x + idx + 1]
                if mode == 26 and c == 0 and n < 32 and not disable_bf:
                    for y in range(n):
                        pred[y][0] = clip3(0, maxv, top[1] + ((left[y] - top[0]) >> 1))
            else:
                ref = {}
                ref[0] = top[0]
                for x in range(1, n + 1):
                    ref[x] = left[x - 1]                              # p[-1][-1 + x]
                if ang < 0:
                    inv = INV_ANGLE[mode]
                    if (n * ang) >> 5 < -1:
                        for x in range((n * ang) >> 5, 0):
                            ref[x] = top[1 + (-1 + ((x * inv + 128) >> 8))]
                else:
                    for x in range(n + 1, 2 * n + 1):
                        ref[x] = left[x - 1]
                for x in range(n):
                    idx = ((x + 1) * ang) >> 5
                    fact = ((x + 1) * ang) & 31
                    for y in range(n):
                        if fact:
                            pred[y][x] = ((32 - fact) * ref[y + idx + 1] + fact * ref[y + idx + 2] + 16) >> 5
                        else:
                            pred[y][x] = ref[y + idx + 1]
                if mode == 10 and c == 0 and n < 32 and not disable_bf:
                    for x in range(n):
                        pred[0][x] = clip3(0, maxv, left[0] + ((top[x + 1] - top[0]) >> 1))
        for y in range(n):
            row = pl[y0 + y] if y0 + y < len(pl) else None
            if row is None:
                continue
            for x in range(n):
                if x0 + x < len(row):
                    row[x0 + x] = pred[y][x]

    # -------------------------------------------------------------- residual coding (7.3.8.11)
    def residual(self, c, x0, y0, log2, pred_mode):
        s, p = self.s, self.p
        cab = self.cab
        ctx = self.ctx
        n = 1 << log2
        cat = s.chroma_array_type
        ts = 0
        if p.transform_skip and not self.cu_bypass and log2 <= p.log2_max_ts:
            ts = cab.decision(ctx["ts_flag"][0 if c == 0 else 1])
        # last significant position
        if c == 0:
            off = 3 * (log2 - 2) + ((log2 - 1) >> 2)
            shift = (log2 + 1) >> 2
        else:
            off = 15
            shift = log2 - 2
        cmax = (log2 << 1) - 1
        px = 0
        while px < cmax and cab.decision(ctx["last_x"][off + (px >> shift)]):
            px += 1
        py = 0
        while py < cmax and cab.decision(ctx["last_y"][off + (py >> shift)]):
            py += 1
        lx = px if px <= 3 else (1 << ((px >> 1) - 1)) * (2 + (px & 1)) + cab.bypass_bits((px >> 1) - 1)
        ly = py if py <= 3 else (1 << ((py >> 1) - 1)) * (2 + (py & 1)) + cab.bypass_bits((py >> 1) - 1)
        # scan
        scan_idx = 0
        if log2 == 2 or (log2 == 3 and (c == 0 or cat == 3)):
            if 6 <= pred_mode <= 14:
                scan_idx = 2
            elif 22 <= pred_mode <= 30:
                scan_idx = 1
        if scan_idx == 2:
            lx, ly = ly, lx
        nsb = 1 << (log2 - 2)
        sb_scan = SCAN[nsb][scan_idx] if nsb > 1 else [(0, 0)]
        pos_scan = SCAN4[scan_idx]
        coeff = [[0] * n for _ in range(n)]
        # find last sub-block / position
        last_sb = next(i for i, (xs, ys) in enumerate(sb_scan) if xs == lx >> 2 and ys == ly >> 2)
        last_pos = next(i for i, (xp, yp) in enumerate(pos_scan) if xp == lx & 3 and yp == ly & 3)
        csbf = {}
        greater1_ctx = 1
        prev_g1_state = None        # c1 carried between sub-blocks
        c1 = 1
        sign_hiding_allowed = p.sign_data_hiding and not self.cu_bypass and not (
            s.implicit_rdpcm and ts and pred_mode in (10, 26))
        for i in range(last_sb, -1, -1):
            xs, ys = sb_scan[i]
            infer_dc = 0
            if i < last_sb and i > 0:
                right = csbf.get((xs + 1, ys), 0) if xs + 1 < nsb else 0
                below = csbf.get((xs, ys + 1), 0) if ys + 1 < nsb else 0
                inc = min(right + below, 1) + (2 if c else 0)
                flag = cab.decision(ctx["csbf"][inc])
                csbf[(xs, ys)] = flag
                infer_dc = 1
            else:
                csbf[(xs, ys)] = 1
            sig = []            # (n position, xC, yC) in reverse scan order
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
                    if k > 0 or not infer_dc:
                        # sig_coeff_flag context
                        if s.ts_context and (ts or self.cu_bypass):
                            sctx = 42 if c == 0 else 16 + 27
                        elif log2 == 2:
                            sctx = CTX_IDX_MAP_4x4[(yc << 2) + xc]
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
                                if log2 == 3:
                                    sctx += 9 if scan_idx == 0 else 15
                                else:
                                    sctx += 21
                            else:
                                sctx += 9 if log2 == 3 else 12
                        if not (s.ts_context and (ts or self.cu_bypass)):
                            sctx = sctx if c == 0 else 27 + sctx
                        f = cab.decision(ctx["sig"][sctx])
                        if f:
                            sig.append(k)
                            infer_dc = 0
                    else:
                        # k == 0 and inferred
                        sig.append(0)
            if not sig:
                continue
            # greater1 / greater2
            ctx_set = 0 if (i == 0 or c > 0) else 2
            if prev_g1_state is not None and prev_g1_state == 0:
                ctx_set += 1
            g1ctx = 1
            absl = [1] * len(sig)
            first_g2 = -1
            for j in range(min(8, len(sig))):
                inc = ctx_set * 4 + g1ctx + (16 if c else 0)
                f = cab.decision(ctx["gt1"][inc])
                absl[j] += f
                if f:
                    g1ctx = 0
                    if first_g2 < 0:
                        first_g2 = j
                elif 0 < g1ctx < 3:
                    g1ctx += 1
            prev_g1_state = g1ctx
            if first_g2 >= 0:
                f = cab.decision(ctx["gt2"][ctx_set + (4 if c else 0)])
                absl[first_g2] += f
            hidden = sign_hiding_allowed and (sig[0] - sig[-1] > 3)
            signs = []
            for j in range(len(sig)):
                if hidden and j == len(sig) - 1:
                    signs.append(None)
                else:
                    signs.append(cab.bypass())
            # remaining
            rice = 0
            num_g1 = 0
            first_coeff2 = 1
            for j in range(len(sig)):
                base = (2 + first_coeff2) if j < 8 else 1
                if absl[j] == base:
                    prefix = 0
                    while cab.bypass():
                        prefix += 1
                        if prefix > 32:
                            raise ValueError("coeff_abs_level_remaining")
                    if prefix <= 3:
                        rem = (prefix << rice) + cab.bypass_bits(rice)
                    else:
                        rem = (((1 << (prefix - 3)) + 3 - 1) << rice) + cab.bypass_bits(prefix - 3 + rice)
                    absl[j] = base + rem
                    if absl[j] > 3 * (1 << rice):
                        rice = min(rice + 1, 4)
                if absl[j] >= 2:
                    first_coeff2 = 0
            sum_abs = 0
            for j, k in enumerate(sig):
                xp, yp = pos_scan[k]
                xc, yc = (xs << 2) + xp, (ys << 2) + yp
                v = absl[j]
                sum_abs += v
                if signs[j] is None:
                    if sum_abs & 1:
                        v = -v
                elif signs[j]:
                    v = -v
                coeff[yc][xc] = v
        self.reconstruct_residual(c, x0, y0, log2, coeff, ts, pred_mode)

    # -------------------------------------------------------------- dequant + inverse transform
    def reconstruct_residual(self, c, x0, y0, log2, coeff, ts, pred_mode):
        s, p, sh = self.s, self.p, self.sh
        n = 1 << log2
        bd = s.bit_depth_y if c == 0 else s.bit_depth_c
        cat = s.chroma_array_type
        if self.cu_bypass:
            res = coeff
        else:
            qpy = self.cu_qpy
            if c == 0:
                qp = qpy + 6 * (s.bit_depth_y - 8)
            else:
                offs = p.cb_qp_offset + sh.cb_qp_offset if c == 1 else p.cr_qp_offset + sh.cr_qp_offset
                qpi = clip3(-6 * (s.bit_depth_c - 8), 57, qpy + offs)
                if cat == 1:
                    if qpi < 30:
                        qpc = qpi
                    elif qpi <= 43:
                        qpc = QPC_TAB[qpi - 30]
                    else:
                        qpc = qpi - 6
                else:
                    qpc = min(qpi, 51)
                qp = qpc + 6 * (s.bit_depth_c - 8)
            bdshift = bd + log2 + 10 - 15
            cmin, cmax = -32768, 32767
            scaling = None
            if s.scaling_list_enabled and not (ts and n > 4):
                sl = p.scaling if p.scaling is not None else s.scaling
                matrix_id = c          # intra: 0..2
                if log2 == 5 and c > 0 and cat != 3:
                    matrix_id = c
                scaling = scaling_factor(sl, log2, matrix_id)
            ls = LEVEL_SCALE[qp % 6] << (qp // 6)
            d = [[0] * n for _ in range(n)]
            for y in range(n):
                for x in range(n):
                    v = coeff[y][x]
                    if v:
                        m = 16 if scaling is None else scaling[(x, y)]
                        d[y][x] = clip3(cmin, cmax, (v * m * ls + (1 << (bdshift - 1))) >> bdshift)
            # transform
            bds = 20 - bd
            if ts:
                tss = 5 + log2
                r = [[(d[y][x] << tss) for x in range(n)] for y in range(n)]
                if s.ts_rotation and n == 4:
                    r = [[r[n - 1 - y][n - 1 - x] for x in range(n)] for y in range(n)]
                res = [[(r[y][x] + (1 << (bds - 1))) >> bds for x in range(n)] for y in range(n)]
            else:
                if n == 4 and c == 0 and not (s.implicit_rdpcm and False):
                    M = DST4
                else:
                    M = DCT[n]
                # vertical (columns)
                e = [[0] * n for _ in range(n)]
                for x in range(n):
                    for y in range(n):
                        acc = 0
                        for k in range(n):
                            if d[k][x]:
                                acc += M[k][y] * d[k][x]
                        e[y][x] = clip3(cmin, cmax, (acc + 64) >> 7)
                res = [[0] * n for _ in range(n)]
                for y in range(n):
                    row = e[y]
                    for x in range(n):
                        acc = 0
                        for k in range(n):
                            if row[k]:
                                acc += M[k][x] * row[k]
                        res[y][x] = (acc + (1 << (bds - 1))) >> bds
        maxv = (1 << bd) - 1
        pl = self.pic.planes[c]
        for y in range(n):
            if y0 + y >= len(pl):
                break
            row = pl[y0 + y]
            for x in range(n):
                if x0 + x < len(row):
                    row[x0 + x] = clip3(0, maxv, row[x0 + x] + res[y][x])

    # -------------------------------------------------------------- in-loop filters
    def finish(self):
        s, p = self.s, self.p
        self.deblock()
        self.sao()
        l, r, t, b = s.conf
        out = Picture()
        out.bit_depth = s.bit_depth_y
        out.bit_depth_c = s.bit_depth_c
        out.chroma_format_idc = s.chroma_format_idc
        out.planes = []
        for c, pl in enumerate(self.pic.planes):
            sw = 1 if c == 0 else s.sub_w
            shh = 1 if c == 0 else s.sub_h
            # conformance window offsets are in chroma units (SubWidthC * offset luma samples)
            x0 = l * s.sub_w // sw
            x1 = len(pl[0]) - r * s.sub_w // sw
            y0 = t * s.sub_h // shh
            y1 = len(pl) - b * s.sub_h // shh
            out.planes.append([row[x0:x1] for row in pl[y0:y1]])
        out.width = len(out.planes[0][0])
        out.height = len(out.planes[0])
        out.vui_colour = getattr(s, "vui_colour", None)
        out.vui_full_range = getattr(s, "vui_full_range", 0)
        return out

    def ctb_rs_of(self, x, y):
        s = self.s
        return (y >> s.log2_ctb) * s.pic_w_ctb + (x >> s.log2_ctb)

    def filter_edge_allowed(self, xq, yq, xp, yp):
        """edge between sample p (xp, yp) and q (xq, yq): slice / tile boundary rules and
        slice_deblocking_filter_disabled_flag of the slice containing q"""
        s, p = self.s, self.p
        rq = self.ctb_rs_of(xq, yq)
        rp = self.ctb_rs_of(xp, yp)
        if self.deblock_off_ctb[rq]:
            return False
        if self.slice_of_ctb[rq] != self.slice_of_ctb[rp] and not self.lf_across_ctb[rq]:
            return False
        if p.tile_id[p.rs2ts[rq]] != p.tile_id[p.rs2ts[rp]] and not p.loop_filter_across_tiles:
            return False
        return True

    def deblock(self):
        s = self.s
        for direction in (0, 1):          # 0: vertical edges, 1: horizontal edges
            pl = self.pic.planes
            new = [[list(row) for row in plane] for plane in pl]
            # luma
            for yy in range(0, s.height, 4):
                for xx in range(0, s.width, 4):
                    if direction == 0:
                        if xx == 0 or (xx & 7) or not self.edge_v[yy >> 2][xx >> 2]:
                            continue
                        if not self.filter_edge_allowed(xx, yy, xx - 1, yy):
                            continue
                        self.luma_edge(new[0], pl[0], xx, yy, 0)
                    else:
                        if yy == 0 or (yy & 7) or not self.edge_h[yy >> 2][xx >> 2]:
                            continue
                        if not self.filter_edge_allowed(xx, yy, xx, yy - 1):
                            continue
                        self.luma_edge(new[0], pl[0], xx, yy, 1)
            # chroma (bS 2 everywhere here: all CUs are intra)
            if s.chroma_array_type != 0:
                for c in (1, 2):
                    cw, ch = len(pl[c][0]), len(pl[c])
                    step_x = 8 * s.sub_w
                    step_y = 8 * s.sub_h
                    for yy in range(0, s.height, 4):
                        for xx in range(0, s.width, 4):
                            if direction == 0:
                                if xx == 0 or xx % step_x or not self.edge_v[yy >> 2][xx >> 2]:
                                    continue
                                if not self.filter_edge_allowed(xx, yy, xx - 1, yy):
                                    continue
                                self.chroma_edge(c, new[c], pl[c], xx, yy, 0)
                            else:
                                if yy == 0 or yy % step_y or not self.edge_h[yy >> 2][xx >> 2]:
                                    continue
                                if not self.filter_edge_allowed(xx, yy, xx, yy - 1):
                                    continue
                                self.chroma_edge(c, new[c], pl[c], xx, yy, 1)
            self.pic.planes = new

    def luma_edge(self, dst, src, xx, yy, hor):
        s = self.s
        bd = s.bit_depth_y
        if hor:
            qpq = self.qpy[yy >> 2][xx >> 2]
            qpp = self.qpy[(yy - 1) >> 2][xx >> 2]
            byp_p = self.bypass[(yy - 1) >> 2][xx >> 2]
            byp_q = self.bypass[yy >> 2][xx >> 2]
        else:
            qpq = self.qpy[yy >> 2][xx >> 2]
            qpp = self.qpy[yy >> 2][(xx - 1) >> 2]
            byp_p = self.bypass[yy >> 2][(xx - 1) >> 2]
            byp_q = self.bypass[yy >> 2][xx >> 2]
        bo, to = self.betatc_ctb[self.ctb_rs_of(xx, yy)]
        qpl = (qpq + qpp + 1) >> 1
        beta = BETA_TAB[clip3(0, 51, qpl + bo)] * (1 << (bd - 8))
        tc = TC_TAB[clip3(0, 53, qpl + 2 * (2 - 1) + to)] * (1 << (bd - 8))
        maxv = (1 << bd) - 1

        def P(i, k):        # p_i,k : i = distance from the edge, k = line
            return src[yy - 1 - i][xx + k] if hor else src[yy + k][xx - 1 - i]

        def Q(i, k):
            return src[yy + i][xx + k] if hor else src[yy + k][xx + i]

        def setP(i, k, v):
            if hor:
                dst[yy - 1 - i][xx + k] = v
            else:
                dst[yy + k][xx - 1 - i] = v

        def setQ(i, k, v):
            if hor:
                dst[yy + i][xx + k] = v
            else:
                dst[yy + k][xx + i] = v
        dp0 = abs(P(2, 0) - 2 * P(1, 0) + P(0, 0))
        dp3 = abs(P(2, 3) - 2 * P(1, 3) + P(0, 3))
        dq0 = abs(Q(2, 0) - 2 * Q(1, 0) + Q(0, 0))
        dq3 = abs(Q(2, 3) - 2 * Q(1, 3) + Q(0, 3))
        d = dp0 + dq0 + dp3 + dq3
        if d >= beta:
            return
        dsam = []
        for k, dpq in ((0, 2 * (dp0 + dq0)), (3, 2 * (dp3 + dq3))):
            dsam.append(dpq < (beta >> 2) and abs(P(3, k) - P(0, k)) + abs(Q(0, k) - Q(3, k)) < (beta >> 3)
                        and abs(P(0, k) - Q(0, k)) < ((5 * tc + 1) >> 1))
        de = 2 if (dsam[0] and dsam[1]) else 1
        dep = (dp0 + dp3) < ((beta + (beta >> 1)) >> 3)
        deq = (dq0 + dq3) < ((beta + (beta >> 1)) >> 3)
        nop = byp_p in (1, 2)
        noq = byp_q in (1, 2)
        for k in range(4):
            p0, p1, p2, p3 = P(0, k), P(1, k), P(2, k), P(3, k)
            q0, q1, q2, q3 = Q(0, k), Q(1, k), Q(2, k), Q(3, k)
            if de == 2:
                np_ = [clip3(p0 - 2 * tc, p0 + 2 * tc, (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3),
                       clip3(p1 - 2 * tc, p1 + 2 * tc, (p2 + p1 + p0 + q0 + 2) >> 2),
                       clip3(p2 - 2 * tc, p2 + 2 * tc, (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3)]
                nq = [clip3(q0 - 2 * tc, q0 + 2 * tc, (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3),
                      clip3(q1 - 2 * tc, q1 + 2 * tc, (p0 + q0 + q1 + q2 + 2) >> 2),
                      clip3(q2 - 2 * tc, q2 + 2 * tc, (p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3)]
                if not nop:
                    for i in range(3):
                        setP(i, k, np_[i])
                if not noq:
                    for i in range(3):
                        setQ(i, k, nq[i])
            else:
                delta = (9 * (q0 - p0) - 3 * (q1 - p1) + 8) >> 4
                if abs(delta) < tc * 10:
                    delta = clip3(-tc, tc, delta)
                    if not nop:
                        setP(0, k, clip3(0, maxv, p0 + delta))
                    if not noq:
                        setQ(0, k, clip3(0, maxv, q0 - delta))
                    if dep and not nop:
                        dp = clip3(-(tc >> 1), tc >> 1, (((p2 + p0 + 1) >> 1) - p1 + delta) >> 1)
                        setP(1, k, clip3(0, maxv, p1 + dp))
                    if deq and not noq:
                        dq = clip3(-(tc >> 1), tc >> 1, (((q2 + q0 + 1) >> 1) - q1 - delta) >> 1)
                        setQ(1, k, clip3(0, maxv, q1 + dq))

    def chroma_edge(self, c, dst, src, xx, yy, hor):
        s, p = self.s, self.p
        bd = s.bit_depth_c
        if hor:
            qpq = self.qpy[yy >> 2][xx >> 2]
            qpp = self.qpy[(yy - 1) >> 2][xx >> 2]
            byp_p = self.bypass[(yy - 1) >> 2][xx >> 2]
            byp_q = self.bypass[yy >> 2][xx >> 2]
        else:
            qpq = self.qpy[yy >> 2][xx >> 2]
            qpp = self.qpy[yy >> 2][(xx - 1) >> 2]
            byp_p = self.bypass[yy >> 2][(xx - 1) >> 2]
            byp_q = self.bypass[yy >> 2][xx >> 2]
        off = p.cb_qp_offset if c == 1 else p.cr_qp_offset
        qpi = ((qpq + qpp + 1) >> 1) + off
        if s.chroma_array_type == 1:
            if qpi < 30:
                qpc = qpi
            elif qpi <= 43:
                qpc = QPC_TAB[qpi - 30]
            else:
                qpc = qpi - 6
        else:
            qpc = min(qpi, 51)
        bo, to = self.betatc_ctb[self.ctb_rs_of(xx, yy)]
        tc = TC_TAB[clip3(0, 53, qpc + 2 + to)] * (1 << (bd - 8))
        maxv = (1 << bd) - 1
        xc, yc = xx // s.sub_w, yy // s.sub_h
        nseg = 4 // (s.sub_h if not hor else s.sub_w)     # chroma samples along the 4-luma segment
        nop = byp_p in (1, 2)
        noq = byp_q in (1, 2)
        for k in range(nseg):
            if hor:
                p0, p1 = src[yc - 1][xc + k], src[yc - 2][xc + k]
                q0, q1 = src[yc][xc + k], src[yc + 1][xc + k]
            else:
                p0, p1 = src[yc + k][xc - 1], src[yc + k][xc - 2]
                q0, q1 = src[yc + k][xc], src[yc + k][xc + 1]
            delta = clip3(-tc, tc, ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3))
            if hor:
                if not nop:
                    dst[yc - 1][xc + k] = clip3(0, maxv, p0 + delta)
                if not noq:
                    dst[yc][xc + k] = clip3(0, maxv, q0 - delta)
            else:
                if not nop:
                    dst[yc + k][xc - 1] = clip3(0, maxv, p0 + delta)
                if not noq:
                    dst[yc + k][xc] = clip3(0, maxv, q0 - delta)

    def sao(self):
        s, p = self.s, self.p
        if not any(self.sao_params):
            return
        src = [[list(r) for r in plane] for plane in self.pic.planes]
        for rs, params in enumerate(self.sao_params):
            if not params:
                continue
            rx, ry = rs % s.pic_w_ctb, rs // s.pic_w_ctb
            for c, prm in enumerate(params):
                if prm is None:
                    continue
                sw = 1 if c == 0 else s.sub_w
                shh = 1 if c == 0 else s.sub_h
                bd = s.bit_depth_y if c == 0 else s.bit_depth_c
                maxv = (1 << bd) - 1
                pl = src[c]
                dst = self.pic.planes[c]
                ph, pw = len(pl), len(pl[0])
                x0 = (rx << s.log2_ctb) // sw
                y0 = (ry << s.log2_ctb) // shh
                w = min(s.ctb_size // sw, pw - x0)
                h = min(s.ctb_size // shh, ph - y0)
                kind, arg, offs = prm
                if kind == "band":
                    shift = bd - 5
                    table = {}
                    for k in range(4):
                        table[(k + arg) & 31] = offs[k]
                    for y in range(y0, y0 + h):
                        for x in range(x0, x0 + w):
                            if self.sao_skip(x * sw, y * shh):
                                continue
                            v = pl[y][x]
                            o = table.get(v >> shift)
                            if o:
                                dst[y][x] = clip3(0, maxv, v + o)
                else:
                    hp = [(-1, 0), (0, -1), (-1, -1), (1, -1)][arg]
                    for y in range(y0, y0 + h):
                        for x in range(x0, x0 + w):
                            if self.sao_skip(x * sw, y * shh):
                                continue
                            ax, ay = x + hp[0], y + hp[1]
                            bx, by = x - hp[0], y - hp[1]
                            if not (0 <= ax < pw and 0 <= ay < ph and 0 <= bx < pw and 0 <= by < ph):
                                continue
                            if not self.sao_nb_ok(x * sw, y * shh, ax * sw, ay * shh) or \
                                    not self.sao_nb_ok(x * sw, y * shh, bx * sw, by * shh):
                                continue
                            v = pl[y][x]
                            e = 2 + (1 if v > pl[ay][ax] else (-1 if v < pl[ay][ax] else 0)) + \
                                (1 if v > pl[by][bx] else (-1 if v < pl[by][bx] else 0))
                            if e in (0, 1, 2):
                                e = 0 if e == 2 else e + 1
                            if e:
                                dst[y][x] = clip3(0, maxv, v + offs[e - 1])

    def sao_skip(self, xl, yl):
        s = self.s
        b = self.bypass[yl >> 2][xl >> 2]
        if b == 2:                         # pcm with pcm_loop_filter_disabled
            return True
        if b == 1 and self.p.transquant_bypass:   # cu_transquant_bypass
            return True
        return False

    def sao_nb_ok(self, x, y, xn, yn):
        s, p = self.s, self.p
        r = self.ctb_rs_of(x, y)
        rn = self.ctb_rs_of(xn, yn)
        if r == rn:
            return True
        # slices: a sample in another slice is used only if the slice that comes later in decoding
        # order allows filtering across slices
        if self.slice_of_ctb[r] != self.slice_of_ctb[rn]:
            later = r if p.rs2ts[r] > p.rs2ts[rn] else rn
            if not self.lf_across_ctb[later]:
                return False
        if p.tile_id[p.rs2ts[r]] != p.tile_id[p.rs2ts[rn]] and not p.loop_filter_across_tiles:
            return False
        return True


def decode(nals, trace=None):
    d = Decoder()
    d.trace = trace
    return d.decode(nals)


if __name__ == "__main__":
    data = open(sys.argv[1], "rb").read()
    pic = decode(split_annexb(data))
    out = bytearray()
    for pl in pic.planes:
        for row in pl:
            if pic.bit_depth > 8:
                for v in row:
                    out += v.to_bytes(2, "little")
            else:
                out += bytes(row)
    if len(sys.argv) > 2:
        open(sys.argv[2], "wb").write(out)
    print(pic.width, pic.height, len(out))
