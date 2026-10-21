#!/usr/bin/env python3
"""JPEG XL encoder model (ISO/IEC 18181-1): the reference of the assembly encoder (jxl_enc*.s).

Lossless modular encoding of 8-bit RGB / RGBA / gray images:

- headers: SizeHeader, ImageMetadata (no XYB, sRGB or gray), all-default TransformData
- frame: one regular modular frame, no loop filters
- colour: YCoCg RCT (rct_type 6)
- prediction: the clamped gradient predictor
- MA tree: one leaf per channel (split on the channel property)
- entropy coding: prefix codes (length-limited Huffman, simple codes for <= 4 symbols), hybrid
  uint config (4, 2, 0), simple context maps
- frames larger than 256 x 256: one AC group section per 256 x 256 group

Every choice is deterministic (first matching U32 distribution, stable Huffman construction), so the
assembly encoder can produce the same bytes.  tools/jxl_ref.py decodes the result.

    python tools/jxl_enc_ref.py IN.png OUT.jxl
"""

import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import jxl_ref as J  # noqa: E402

V, B = J.V, J.B
GROUP_DIM = 256                 # group_size_shift 1
HU_SE, HU_MSB, HU_LSB = 4, 2, 0  # hybrid uint config of every histogram
PREFIX_LIMIT = 15
CL_LIMIT = 5
# static code of the code length code lengths 0..5: (LSB-first pattern, bits)
CL_STATIC = {0: (0, 2), 1: (7, 4), 2: (3, 3), 3: (2, 2), 4: (1, 2), 5: (15, 4)}


# ====================================================================== bit writer

class BW:
    """LSB-first bit writer"""

    def __init__(self):
        self.out = bytearray()
        self.acc = 0
        self.n = 0

    def f(self, v, n):
        assert 0 <= v < (1 << n) or n == 0 and v == 0, (v, n)
        self.acc |= v << self.n
        self.n += n
        while self.n >= 8:
            self.out.append(self.acc & 255)
            self.acc >>= 8
            self.n -= 8

    def bool(self, v):
        self.f(1 if v else 0, 1)

    def u32(self, v, dists):
        """the first distribution that can hold v"""
        for sel, d in enumerate(dists):
            if d[0] == 'v':
                if v == d[1]:
                    self.f(sel, 2)
                    return
            elif d[2] <= v < d[2] + (1 << d[1]):
                self.f(sel, 2)
                self.f(v - d[2], d[1])
                return
        raise ValueError("U32 value %d" % v)

    def u64(self, v):
        if v == 0:
            self.f(0, 2)
        elif v <= 16:
            self.f(1, 2)
            self.f(v - 1, 4)
        elif v <= 272:
            self.f(2, 2)
            self.f(v - 17, 8)
        else:
            raise ValueError("U64 value")

    def enum(self, v):
        self.u32(v, (V(0), V(1), B(4, 2), B(6, 18)))

    def pad(self):
        if self.n:
            self.f(0, 8 - self.n)

    def bits(self):
        return len(self.out) * 8 + self.n

    def data(self):
        assert self.n == 0
        return bytes(self.out)


def varlen_u16(bw, v):
    if v == 0:
        bw.f(0, 1)
    elif v == 1:
        bw.f(1, 1)
        bw.f(0, 4)
    else:
        n = J.floor_log2(v)
        bw.f(1, 1)
        bw.f(n, 4)
        bw.f(v - (1 << n), n)


def pack_signed(v):
    return 2 * v if v >= 0 else -2 * v - 1


# ====================================================================== hybrid uint

def hybrid_token(v):
    """(token, extra bits, number of extra bits) of HybridUintConfig( HU_SE, HU_MSB, HU_LSB )"""
    split = 1 << HU_SE
    if v < split:
        return v, 0, 0
    top = J.floor_log2(v)
    n = top - HU_MSB - HU_LSB
    m = (v >> (n + HU_LSB)) & ((1 << HU_MSB) - 1)
    low = v & ((1 << HU_LSB) - 1)
    token = split + ((top - HU_SE) << (HU_MSB + HU_LSB)) + (m << HU_LSB) + low
    bits = (v >> HU_LSB) & ((1 << n) - 1)
    return token, bits, n


# ====================================================================== prefix codes

def huffman_lengths(counts, limit):
    """code lengths of a Huffman code over the used symbols, at most `limit` bits: repeated with
    every count raised to at least 1, 2, 4, ... until the depth fits (deterministic two-queue
    construction: leaves sorted by (count, symbol); on equal weights the leaf queue goes first)"""
    syms = [s for s, c in enumerate(counts) if c]
    lengths = [0] * len(counts)
    if len(syms) <= 1:
        return lengths
    floor = 1
    while True:
        leaves = sorted((max(counts[s], floor), s) for s in syms)
        # node: (weight, children or symbol)
        lq = [(w, ('L', s)) for w, s in leaves]
        iq = []
        li = ii = 0

        def take():
            nonlocal li, ii
            if li < len(lq) and (ii >= len(iq) or lq[li][0] <= iq[ii][0]):
                li += 1
                return lq[li - 1]
            ii += 1
            return iq[ii - 1]
        for _ in range(len(syms) - 1):
            a = take()
            b = take()
            iq.append((a[0] + b[0], ('N', a, b)))
        root = iq[-1]
        depth = {}
        stack = [(root, 0)]
        while stack:
            node, d = stack.pop()
            if node[1][0] == 'L':
                depth[node[1][1]] = d
            else:
                stack.append((node[1][1], d + 1))
                stack.append((node[1][2], d + 1))
        if max(depth.values()) <= limit:
            for s, d in depth.items():
                lengths[s] = d
            return lengths
        floor *= 2


def canonical_codes(lengths):
    """symbol -> (code, length), the decoder's assignment: sorted by (length, symbol), MSB first"""
    order = sorted((l, s) for s, l in enumerate(lengths) if l)
    codes = {}
    code = 0
    prev = order[0][0] if order else 0
    for l, s in order:
        code <<= (l - prev)
        prev = l
        codes[s] = (code, l)
        code += 1
    return codes


def put_code(bw, code, length):
    """a prefix code word, most significant bit first"""
    for i in range(length - 1, -1, -1):
        bw.f((code >> i) & 1, 1)


def write_prefix(bw, lengths, single=0):
    """ReadPrefixCode inverse for an alphabet of len( lengths ) >= 2 symbols (all lengths 0: the one
    symbol `single`)"""
    a = len(lengths)
    used = [s for s in range(a) if lengths[s]]
    if len(used) <= 4:
        maxbits = J.floor_log2(a - 1) + 1
        bw.f(1, 2)
        if not used:
            used = [single]
        n = len(used)
        if n == 1:
            syms = used
        else:
            syms = sorted(used, key=lambda s: (lengths[s], s))
        bw.f(n - 1, 2)
        for s in syms:
            bw.f(s, maxbits)
        if n == 4:
            bw.f(1 if lengths[syms[0]] == 1 else 0, 1)
        return
    last = max(used)
    seq = lengths[:last + 1]
    cl_counts = [0] * 18
    for l in seq:
        cl_counts[l] += 1
    cl = huffman_lengths(cl_counts, CL_LIMIT)
    nz = [s for s in range(18) if cl_counts[s]]
    if len(nz) == 1:
        cl = [0] * 18
        cl[nz[0]] = 1
    order = J.CL_ORDER
    hskip = 0
    if cl[order[0]] == 0 and cl[order[1]] == 0:
        hskip = 3 if cl[order[2]] == 0 else 2
    bw.f(hskip, 2)
    space = 32
    for i in range(hskip, 18):
        v = cl[order[i]]
        pat, nb = CL_STATIC[v]
        bw.f(pat, nb)
        if v:
            space -= 32 >> v
            if space == 0:
                break
    if len(nz) == 1:
        return                                  # single code length symbol: 0 bits each
    codes = canonical_codes(cl)
    for l in seq:
        put_code(bw, *codes[l])


# ====================================================================== ANS

ANS_TAB = 4096
ANS_SHIFT = 13                  # histogram precision: every count exact
HIST_CODE = {}                  # log count -> (LSB-first pattern, bits) of the static ReadHistogram code
for _idx, (_nb, _lc) in enumerate(J.HIST_HUFF):
    if _lc not in HIST_CODE:
        HIST_CODE[_lc] = (_idx & ((1 << _nb) - 1), _nb)


def ans_normalize(counts):
    """counts -> frequencies summing to 4096, every used symbol >= 1: floor( c * 4096 / total ),
    raised to 1; the difference goes to the first most frequent symbol, while the sum is too big
    the first largest frequency loses 1"""
    total = sum(counts)
    f = [0 if c == 0 else max(1, c * ANS_TAB // total) for c in counts]
    top = max(range(len(counts)), key=lambda i: counts[i])
    f[top] += ANS_TAB - sum(f)
    while f[top] < 1 or sum(f) > ANS_TAB:
        if f[top] < 1:
            f[top] = 1
        big = max(range(len(f)), key=lambda i: f[i])
        f[big] -= 1
    return f


def write_ans_histogram(bw, f):
    """ReadHistogram inverse (alphabet of len( f ) symbols, sum 4096)"""
    used = [i for i, v in enumerate(f) if v]
    if len(used) <= 2:
        bw.f(1, 1)                              # simple
        bw.f(len(used) - 1, 1)
        for sym in used:
            varlen_u8(bw, sym)
        if len(used) == 2:
            bw.f(f[used[0]], 12)
        return
    bw.f(0, 1)
    bw.f(0, 1)                                  # not flat
    bw.f(7, 3)                                  # shift 13: log 3 (unary 1, 1, 1), then 6
    bw.f(6, 3)
    n = max(len(f), 3)
    f = list(f) + [0] * (n - len(f))
    varlen_u8(bw, n - 3)
    lc = [0 if v == 0 else J.floor_log2(v) + 1 for v in f]
    omit = max(range(n), key=lambda i: lc[i])
    for v in lc:
        pat, nb = HIST_CODE[v]
        bw.f(pat, nb)
    for i in range(n):
        if i == omit or lc[i] <= 1:
            continue
        bw.f(f[i] - (1 << (lc[i] - 1)), lc[i] - 1)


def varlen_u8(bw, v):
    if v == 0:
        bw.f(0, 1)
    elif v == 1:
        bw.f(1, 1)
        bw.f(0, 3)
    else:
        n = J.floor_log2(v)
        bw.f(1, 1)
        bw.f(n, 3)
        bw.f(v - (1 << n), n)


def ans_slots(f, log_alpha):
    """slot (0 .. 4095) of every (symbol, offset): inverse of the decoder's alias table"""
    tab = J.init_alias(f, J.ANS_LOG_TAB_SIZE, log_alpha)
    log_entry = J.ANS_LOG_TAB_SIZE - log_alpha
    rev = [[0] * v for v in f]
    for res in range(ANS_TAB):
        i = res >> log_entry
        pos = res & ((1 << log_entry) - 1)
        cut, right, f0, off1, f1 = tab[i]
        if pos >= cut:
            sym, off = right, off1 + pos
        else:
            sym, off = i, pos
        rev[sym][off] = res
    return rev


# ====================================================================== histograms

class Histos:
    """the tokens of one entropy-coded stream set: contexts -> clusters -> prefix codes; with
    fixed_map the contexts go to the given clusters (every cluster 0 .. max occurs)"""

    def __init__(self, nctx, fixed_map=None, ans=True):
        self.nctx = nctx
        self.fixed = fixed_map
        self.ans = ans
        self.counts = [[0] * 1 for _ in range(nctx)]

    def add(self, ctx, token):
        c = self.counts[ctx]
        if token >= len(c):
            c.extend([0] * (token + 1 - len(c)))
        c[token] += 1

    def finish(self):
        """context map: used contexts get clusters in order, unused ones cluster 0 (or the fixed
        map: summed counts per cluster, an empty cluster holds the symbol 0)"""
        if self.fixed is not None:
            self.ctx_map = list(self.fixed)
            self.clusters = [[0] for _ in range(max(self.ctx_map) + 1)]
            for i in range(self.nctx):
                acc = self.clusters[self.ctx_map[i]]
                for t, v in enumerate(self.counts[i]):
                    if v:
                        if t >= len(acc):
                            acc.extend([0] * (t + 1 - len(acc)))
                        acc[t] += v
            for k, c in enumerate(self.clusters):
                if not any(c):
                    self.clusters[k] = [1]
        else:
            self.ctx_map = [0] * self.nctx
            self.clusters = []
            for i in range(self.nctx):
                if any(self.counts[i]):
                    self.ctx_map[i] = len(self.clusters)
                    self.clusters.append(self.counts[i])
            if not self.clusters:
                self.clusters.append([1])
        if len(self.clusters) > 8:
            raise ValueError("more than 8 clusters (no complex context map yet)")
        if self.ans:
            self.freqs = [ans_normalize(c) for c in self.clusters]
            amax = max(len(c) for c in self.clusters)
            self.log_alpha = max(5, J.ceil_log2(amax))
            self.slots = [ans_slots(fr, self.log_alpha) for fr in self.freqs]
            return
        self.lengths = []
        self.codes = []
        for c in self.clusters:
            while len(c) > 1 and c[-1] == 0:
                c.pop()
            l = huffman_lengths(c, PREFIX_LIMIT)
            self.lengths.append(l)
            self.codes.append(canonical_codes(l) if len(c) > 1 and sum(1 for v in l if v) > 1 else None)

    def write(self, bw):
        if self.ans:
            self.write_ans(bw)
            return
        bw.bool(False)                          # LZ77
        if self.nctx > 1:
            write_context_map(bw, self.ctx_map)
        bw.bool(True)                           # prefix codes
        for _ in self.clusters:                 # HybridUintConfig per histogram (log_alpha 15)
            bw.f(HU_SE, 4)
            bw.f(HU_MSB, J.ceil_log2(HU_SE + 1))
            bw.f(HU_LSB, J.ceil_log2(HU_SE - HU_MSB + 1))
        for c in self.clusters:
            varlen_u16(bw, len(c) - 1)
        for c, l in zip(self.clusters, self.lengths):
            if len(c) > 1:
                write_prefix(bw, l, max(range(len(c)), key=lambda t: c[t]))

    def write_ans(self, bw):
        bw.bool(False)                          # LZ77
        if self.nctx > 1:
            write_context_map(bw, self.ctx_map)
        bw.bool(False)                          # ANS
        bw.f(self.log_alpha - 5, 2)
        se_bits = J.ceil_log2(self.log_alpha + 1)
        for _ in self.clusters:
            bw.f(HU_SE, se_bits)
            bw.f(HU_MSB, J.ceil_log2(HU_SE + 1))
            bw.f(HU_LSB, J.ceil_log2(HU_SE - HU_MSB + 1))
        for fr in self.freqs:
            write_ans_histogram(bw, fr)

    def stream(self, bw):
        return AnsStream(self, bw) if self.ans else PrefixStream(self, bw)

    def put(self, bw, ctx, v):
        """HybridUint value v in context ctx"""
        tok, bits, n = hybrid_token(v)
        h = self.ctx_map[ctx]
        codes = self.codes[h]
        if codes is not None:
            put_code(bw, *codes[tok])
        bw.f(bits, n)


CTXMAP_SIMPLE_MAX = 16


def write_context_map(bw, cm):
    """DecodeContextMap inverse: up to CTXMAP_SIMPLE_MAX entries the simple form (bits per entry),
    larger maps move-to-front transformed and entropy coded (one ANS context)"""
    n = len(cm)
    if n <= CTXMAP_SIMPLE_MAX:
        bpe = J.ceil_log2(max(cm) + 1)
        bw.f(1, 1)
        bw.f(bpe, 2)
        if bpe:
            for v in cm:
                bw.f(v, bpe)
        return
    bw.f(0, 1)
    bw.f(1, 1)                                  # move-to-front
    mtf = list(range(256))
    vals = []
    for v in cm:
        i = mtf.index(v)
        vals.append(i)
        if i:
            del mtf[i]
            mtf.insert(0, v)
    hs = Histos(1)
    for v in vals:
        hs.add(0, hybrid_token(v)[0])
    hs.finish()
    hs.write(bw)
    st = hs.stream(bw)
    for v in vals:
        st.put(0, v)
    st.flush()


class PrefixStream:
    """tokens written immediately with the prefix codes"""

    def __init__(self, hs, bw):
        self.hs, self.bw = hs, bw

    def put(self, ctx, v):
        self.hs.put(self.bw, ctx, v)

    def flush(self):
        pass


class AnsStream:
    """tokens collected, then rANS-coded backwards: the 32-bit final state, then per token the
    16-bit renormalization chunk the decoder reads after it (if any) and its extra bits"""

    def __init__(self, hs, bw):
        self.hs, self.bw = hs, bw
        self.toks = []

    def put(self, ctx, v):
        tok, bits, n = hybrid_token(v)
        self.toks.append((self.hs.ctx_map[ctx], tok, bits, n))

    def flush(self):
        hs = self.hs
        state = J.ANS_SIGNATURE << 16
        chunks = [None] * len(self.toks)
        for i in range(len(self.toks) - 1, -1, -1):
            h, tok, _, _ = self.toks[i]
            freq = hs.freqs[h][tok]
            if (state >> (32 - J.ANS_LOG_TAB_SIZE)) >= freq:
                chunks[i] = state & 0xFFFF
                state >>= 16
            state = ((state // freq) << J.ANS_LOG_TAB_SIZE) + hs.slots[h][tok][state % freq]
        bw = self.bw
        bw.f(state, 32)
        for i, (_, _, bits, n) in enumerate(self.toks):
            if chunks[i] is not None:
                bw.f(chunks[i], 16)
            bw.f(bits, n)
        self.toks = []


# ====================================================================== modular

def rct_ycocg(r, g, b):
    co = r - b
    tmp = b + (co >> 1)
    cg = g - tmp
    y = tmp + (cg >> 1)
    return y, co, cg


def residuals(ch):
    """clamped gradient residuals of a channel (list of rows), in decode order"""
    out = []
    h = len(ch)
    w = len(ch[0]) if h else 0
    for y in range(h):
        row = ch[y]
        prev = ch[y - 1] if y else None
        for x in range(w):
            left = row[x - 1] if x else (prev[x] if y else 0)
            top = prev[x] if y else left
            topleft = prev[x - 1] if (x and y) else left
            out.append(row[x] - J.clamped_gradient(left, top, topleft))
    return out


def tree_nodes(nch):
    """MA tree in decode (BFS) order: splits on property 0 (channel) down to one leaf per channel.
    Node: ('split', value) or ('leaf', channel)"""
    def build(lo, hi):
        if lo == hi:
            return ('leaf', lo)
        mid = (lo + hi) // 2               # channel > mid -> left (first child)
        return ('split', mid, build(mid + 1, hi), build(lo, mid))
    root = build(0, nch - 1)
    order = []
    q = [root]
    while q:
        n = q.pop(0)
        order.append(n)
        if n[0] == 'split':
            q.append(n[2])
            q.append(n[3])
    leaf_of = {}
    for n in order:
        if n[0] == 'leaf':
            leaf_of[n[1]] = len(leaf_of)
    return order, leaf_of


def write_tree(bw, nodes):
    """the tree with its own histograms (6 contexts)"""
    toks = []
    for n in nodes:
        if n[0] == 'split':
            toks += [(1, 0 + 1), (0, pack_signed(n[1]))]     # property 0 -> prop + 1 = 1
        else:
            toks += [(1, 0), (2, 5), (3, 0), (4, 0), (5, 0)]  # leaf: gradient, offset 0, multiplier 1
    hs = Histos(6)
    for ctx, v in toks:
        hs.add(ctx, hybrid_token(v)[0])
    hs.finish()
    hs.write(bw)
    st = hs.stream(bw)
    for ctx, v in toks:
        st.put(ctx, v)
    st.flush()


def group_header(bw, use_global, rct):
    bw.bool(use_global)
    bw.bool(True)                               # default weighted predictor header
    if rct:
        bw.u32(1, (V(0), V(1), B(4, 2), B(8, 18)))
        bw.u32(0, (V(0), V(1), V(2), V(3)))     # RCT
        bw.u32(0, (B(3), B(6, 8), B(10, 72), B(13, 1096)))   # begin_c
        bw.u32(6, (V(6), B(2), B(4, 2), B(6, 10)))          # YCoCg
    else:
        bw.u32(0, (V(0), V(1), B(4, 2), B(8, 18)))


# ====================================================================== headers

def size_header(bw, w, h):
    small = w <= 256 and h <= 256 and w % 8 == 0 and h % 8 == 0
    bw.bool(small)
    if small:
        bw.f(h // 8 - 1, 5)
        bw.f(0, 3)
        bw.f(w // 8 - 1, 5)
    else:
        d = (B(9, 1), B(13, 1), B(18, 1), B(30, 1))
        bw.u32(h, d)
        bw.f(0, 3)
        bw.u32(w, d)


def image_metadata(bw, gray, alpha):
    bw.bool(False)                              # all_default (would mean XYB)
    bw.bool(False)                              # extra_fields
    bw.bool(False)                              # bit depth: integer
    bw.u32(8, (V(8), V(10), V(12), B(6, 1)))
    bw.bool(True)                               # modular_16bit_buffers
    bw.u32(1 if alpha else 0, (V(0), V(1), B(4, 2), B(12, 1)))
    if alpha:
        bw.bool(True)                           # all_default: 8-bit unassociated alpha
    bw.bool(False)                              # xyb_encoded
    if not gray:
        bw.bool(True)                           # colour encoding all_default: sRGB
    else:
        bw.bool(False)
        bw.bool(False)                          # want_icc
        bw.enum(J.ColorEncoding.GRAY)
        bw.enum(1)                              # white point D65
        bw.bool(False)                          # have_gamma
        bw.enum(J.TF_SRGB)
        bw.enum(1)                              # rendering intent: relative
    bw.u64(0)                                   # extensions


def frame_header(bw, nec):
    bw.bool(False)                              # all_default (VarDCT)
    bw.u32(0, (V(0), V(1), V(2), V(3)))         # regular frame
    bw.bool(True)                               # modular
    bw.u64(0)                                   # flags
    bw.bool(False)                              # no YCbCr
    bw.u32(1, (V(1), V(2), V(4), V(8)))         # upsampling
    for _ in range(nec):
        bw.u32(1, (V(1), V(2), V(4), V(8)))
    bw.f(1, 2)                                  # group_size_shift: 256
    bw.u32(1, (V(1), V(2), V(3), B(3, 4)))      # passes
    bw.bool(False)                              # custom size
    for _ in range(1 + nec):                    # blending: replace
        bw.u32(0, (V(0), V(1), V(2), B(2, 3)))
    bw.bool(True)                               # is_last
    bw.u32(0, (V(0), B(4), B(5, 16), B(10, 48)))   # name
    bw.bool(False)                              # loop filter not all_default
    bw.bool(False)                              # no gaborish
    bw.f(0, 2)                                  # no EPF
    bw.u64(0)                                   # loop filter extensions
    bw.u64(0)                                   # frame extensions


# ====================================================================== encoder

def encode_lossless(w, h, rgba):
    """rgba: bytes, 4 per pixel (R, G, B, A), row by row -> JPEG XL codestream"""
    px = [rgba[i::4] for i in range(4)]
    gray = px[0] == px[1] == px[2]
    alpha = any(v != 255 for v in px[3])
    rows = lambda p: [list(p[y * w:(y + 1) * w]) for y in range(h)]
    if gray:
        chans = [rows(px[0])]
    else:
        r, g, b = rows(px[0]), rows(px[1]), rows(px[2])
        yy, co, cg = [], [], []
        for y in range(h):
            t = [rct_ycocg(r[y][x], g[y][x], b[y][x]) for x in range(w)]
            yy.append([v[0] for v in t])
            co.append([v[1] for v in t])
            cg.append([v[2] for v in t])
        chans = [yy, co, cg]
    if alpha:
        chans.append(rows(px[3]))
    nch = len(chans)
    nodes, leaf_of = tree_nodes(nch)
    xg, yg = -(-w // GROUP_DIM), -(-h // GROUP_DIM)
    groups = [(gx * GROUP_DIM, gy * GROUP_DIM) for gy in range(yg) for gx in range(xg)]

    # residual tokens of every group, channel by channel
    def group_res(x0, y0):
        res = []
        for c, ch in enumerate(chans):
            sub = [row[x0:x0 + GROUP_DIM] for row in ch[y0:y0 + GROUP_DIM]]
            res.append((leaf_of[c], residuals(sub)))
        return res
    gres = [group_res(x0, y0) for x0, y0 in groups]
    hs = Histos(len(leaf_of))
    for res in gres:
        for leaf, rs in res:
            for v in rs:
                hs.add(leaf, hybrid_token(pack_signed(v))[0])
    hs.finish()

    def put_res(bw, res):
        st = hs.stream(bw)
        for leaf, rs in res:
            for v in rs:
                st.put(leaf, pack_signed(v))
        st.flush()

    # DC global: dc_quant default, global tree + histograms, global modular header
    gl = BW()
    gl.bool(True)                               # default DC quantization
    gl.f(1, 1)                                  # global tree
    write_tree(gl, nodes)
    hs.write(gl)
    group_header(gl, True, not gray)
    single = len(groups) == 1
    if single:
        put_res(gl, gres[0])
    gl.pad()
    sections = [gl.data()]
    if not single:
        xb = -(-w // 8)
        yb = -(-h // 8)
        ndg = -(-xb // GROUP_DIM) * -(-yb // GROUP_DIM)
        sections += [b""] * ndg                 # DC groups
        sections.append(b"")                    # AC global
        for res in gres:
            g = BW()
            group_header(g, True, False)
            put_res(g, res)
            g.pad()
            sections.append(g.data())

    out = BW()
    out.f(0xFF, 8)
    out.f(0x0A, 8)
    size_header(out, w, h)
    image_metadata(out, gray, alpha)
    out.bool(True)                              # TransformData all_default
    out.pad()
    frame_header(out, 1 if alpha else 0)
    out.f(0, 1)                                 # TOC: no permutation
    out.pad()
    for s in sections:
        out.u32(len(s), (B(10), B(14, 1024), B(22, 17408), B(30, 4211712)))
    out.pad()
    return out.data() + b"".join(sections)


# ====================================================================== VarDCT (lossy)
#
# Stage A: XYB, DCT8 everywhere, one quantizer for the whole frame, library quantization tables, no
# loop filters, colour correlation only as the default (B predicted by Y), natural coefficient
# orders, three block contexts (one per channel), AC contexts in 7 fixed clusters.  All float math
# is numpy float32 in a fixed order (the assembly encoder repeats it).

F32 = J.F32
NP = J.np
OPSIN_M = [[0.30, 1.0 - 0.078 - 0.30, 0.078], [0.23, 1.0 - 0.078 - 0.23, 0.078],
           [0.24342268924547819, 0.20476744424496821, 1.0 - 0.24342268924547819 - 0.20476744424496821]]
OPSIN_BIAS = 0.0037930732552754493
THR_Y = [0.56, 0.62, 0.62, 0.62]
THR_XB = [0.58, 0.62, 0.62, 0.62]
K_AC_QUANT = 0.725
K_DC_QUANT = 1.095924047623553
K_DC_POW = 0.83


def srgb_lut():
    """sRGB 8-bit -> linear (float32)"""
    out = []
    for v in range(256):
        x = v / 255.0
        out.append(x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4)
    return NP.array(out, F32)


def cbrt_f32(a):
    """cube root of a >= 0 (float32 array): bits / 3 + 0x2a5137a0, three Newton steps
    x = (2 x + a / (x x)) * (1 / 3); 0 for a = 0"""
    a = NP.asarray(a, F32)
    bits = a.view(NP.uint32).astype(NP.uint64)
    x = ((bits // 3 + 0x2a5137a0) & 0xFFFFFFFF).astype(NP.uint32).view(F32)
    third = F32(1.0 / 3.0)
    safe = NP.where(a > 0, a, F32(1))
    x = NP.where(a > 0, x, F32(1))
    for _ in range(3):
        x = ((x + x) + safe / (x * x)) * third
    return NP.where(a > 0, x, F32(0)).astype(F32)


def to_xyb(w, h, rgba):
    """sRGB 8-bit -> XYB planes (float32, h x w)"""
    lut = srgb_lut()
    px = NP.frombuffer(rgba, NP.uint8).reshape(h, w, 4)
    lin = [lut[px[..., c]] for c in range(3)]
    m = [[F32(v) for v in row] for row in OPSIN_M]
    bias = F32(OPSIN_BIAS)
    cb = cbrt_f32(NP.array([bias], F32))[0]
    mixed = []
    for i in range(3):
        v = ((m[i][0] * lin[0] + m[i][1] * lin[1]) + m[i][2] * lin[2]) + bias
        v = NP.maximum(v, F32(0))
        mixed.append(cbrt_f32(v) - cb)
    x = (mixed[0] - mixed[1]) * F32(0.5)
    y = (mixed[0] + mixed[1]) * F32(0.5)
    return [x.astype(F32), y.astype(F32), mixed[2].astype(F32)]


def distance_of_quality(q):
    """JxlEncoderDistanceFromQuality"""
    if q >= 100:
        return 0.0
    if q >= 30:
        return 0.1 + (100 - q) * 0.09
    return 53.0 / 3000.0 * q * q - 23.0 / 20.0 * q + 25.0


def quant_params(distance):
    """(global_scale, quant_dc, qf): InitialQuantDC, kAcQuant / d, ComputeGlobalScaleAndQuant"""
    d = F32(distance)
    p = J.fast_powf(NP.array([d * F32(1.0 / 0.3)], F32), NP.array([K_DC_POW], F32))[0]
    dcd = max(F32(0.5) * d, min(d, F32(0.3) * p))
    quant_dc = min(F32(K_DC_QUANT) / dcd, F32(50))
    quant_ac = F32(K_AC_QUANT) / d
    scale = F32(65536) * quant_ac / F32(5)
    scale = min(max(scale, F32(1)), F32(1 << 15))
    gs = int(scale)
    sq = int(quant_dc * F32(4096 * 1.6))
    if gs > sq:
        gs = max(sq, 1)
    inv_gs = F32(65536.0 / gs)
    qdc = int(min(F32(1 << 16), quant_dc * inv_gs + F32(0.5)))
    qf = int(quant_ac * inv_gs + F32(0.5))
    qf = max(1, min(256, qf))
    return gs, max(qdc, 1), qf


def ac_cluster_map(nctx_blk):
    """fixed AC context clusters: 0 = non-zero counts; coefficients 1 + prev * 3 + band of
    v = NZ_CTX + FREQ_CTX (< 40, < 120, else)"""
    m = [0] * (nctx_blk * (J.NONZERO_BUCKETS + J.ZERO_DENSITY_CONTEXTS))
    base = nctx_blk * J.NONZERO_BUCKETS
    for b in range(nctx_blk):
        for i in range(J.ZERO_DENSITY_CONTEXTS):
            prev = i & 1
            v = i >> 1
            band = 0 if v < 40 else 1 if v < 120 else 2
            m[base + J.ZERO_DENSITY_CONTEXTS * b + i] = 1 + prev * 3 + band
    return m


def write_modular_local(bw, chans):
    """a modular sub-image with its own tree (one leaf per channel, gradient) and histograms:
    group header (local tree, default WP, no transforms), tree, histograms, residuals"""
    bw.bool(False)
    bw.bool(True)
    bw.u32(0, (V(0), V(1), B(4, 2), B(8, 18)))
    nch = len(chans)
    nodes, leaf_of = tree_nodes(nch)
    write_tree(bw, nodes)
    hs = Histos(len(leaf_of))
    res = [(leaf_of[c], residuals(ch)) for c, ch in enumerate(chans)]
    for leaf, rs in res:
        for v in rs:
            hs.add(leaf, hybrid_token(pack_signed(v))[0])
    hs.finish()
    hs.write(bw)
    st = hs.stream(bw)
    for leaf, rs in res:
        for v in rs:
            st.put(leaf, pack_signed(v))
    st.flush()


def vardct_blocks(planes, xb, yb, distance):
    """quantized DC (3 planes of ints, order X, Y, B) and AC (yb, xb, 3, 64) of every block"""
    gs, qdc, qf = quant_params(distance)
    inv_gs = F32(65536.0 / gs)
    dcq = [F32(1.0 / 4096), F32(1.0 / 512), F32(1.0 / 256)]
    iqdc = inv_gs / F32(qdc)
    mul_dc = [iqdc * dcq[c] for c in range(3)]
    sds = inv_gs / F32(qf)
    x_dm = F32(float(F32(1) / F32(1.25)) ** (3 - 2))
    b_dm = F32(float(F32(1) / F32(1.25)) ** (2 - 2))
    mats = J.quant_table(J.QUANT_LIBRARY[0], 0)
    step = [mats[0] * (sds * x_dm), mats[1] * sds, mats[2] * (sds * b_dm)]
    qbias = [F32(v) for v in J.DEFAULT_QUANT_BIAS]
    quad = [(2 if k // 8 >= 4 else 0) + (1 if k % 8 >= 4 else 0) for k in range(64)]
    thr_y = NP.array([THR_Y[i] for i in quad], F32)
    thr_xb = NP.array([THR_XB[i] for i in quad], F32)

    def qround(val, thr):
        r = NP.rint(val)
        return NP.where(NP.abs(val) >= thr, r, F32(0)).astype(NP.int64)
    dcq_img = [NP.zeros((yb, xb), NP.int64) for _ in range(3)]
    acq = NP.zeros((yb, xb, 3, 64), NP.int64)
    for by in range(yb):
        for bx in range(xb):
            co = [J.dct2d(planes[c][by * 8:by * 8 + 8, bx * 8:bx * 8 + 8], 8, 8).reshape(64) for c in range(3)]
            qy = int(NP.rint(co[1][0] / mul_dc[1]))
            ry = F32(qy) * mul_dc[1]
            qx = int(NP.rint(co[0][0] / mul_dc[0]))
            qb = int(NP.rint((co[2][0] - ry) / mul_dc[2]))
            dcq_img[1][by, bx], dcq_img[0][by, bx], dcq_img[2][by, bx] = qy, qx, qb
            vy = qround(co[1] / step[1], thr_y)
            vy[0] = 0
            dy = J.adjust_quant_bias(vy, 1, qbias) * step[1]
            vx = qround(co[0] / step[0], thr_xb)
            vx[0] = 0
            vb = qround((co[2] - dy) / step[2], thr_xb)
            vb[0] = 0
            acq[by, bx, 0], acq[by, bx, 1], acq[by, bx, 2] = vx, vy, vb
    return (gs, qdc, qf), dcq_img, acq


def encode_vardct(w, h, rgba, distance):
    """lossy VarDCT codestream (stage A)"""
    alpha = any(v != 255 for v in rgba[3::4])
    xyb = to_xyb(w, h, rgba)
    xb, yb = -(-w // 8), -(-h // 8)
    planes = []
    for p in xyb:                               # pad by edge replication to whole blocks
        q = NP.empty((yb * 8, xb * 8), F32)
        q[:h, :w] = p
        q[:h, w:] = p[:, w - 1:w]
        q[h:, :] = q[h - 1:h, :]
        planes.append(q)
    (gs, qdc, qf), dcq_img, acq = vardct_blocks(planes, xb, yb, distance)
    order = J.natural_order(J.ACS_DCT)
    xg, yg = -(-w // GROUP_DIM), -(-h // GROUP_DIM)
    ng = xg * yg
    xdg, ydg = -(-xb // GROUP_DIM), -(-yb // GROUP_DIM)
    ndg = xdg * ydg
    nctx_blk = 3
    num_ac_ctx = nctx_blk * (J.NONZERO_BUCKETS + J.ZERO_DENSITY_CONTEXTS)
    blk_map = [0] * 13 + [1] * 13 + [2] * 13            # Y, X, B block contexts

    def group_tokens(g):
        gx, gy = g % xg, g // xg
        x0, y0 = gx * 32, gy * 32
        gw, gh = min(32, xb - x0), min(32, yb - y0)
        nz = [[[0] * 32 for _ in range(32)] for _ in range(3)]
        toks = []
        for by in range(gh):
            for bx in range(gw):
                for c in (1, 0, 2):
                    q = acq[y0 + by, x0 + bx, c]
                    row, top = nz[c][by], (nz[c][by - 1] if by else None)
                    if bx == 0:
                        pred = 32 if top is None else top[0]
                    elif top is None:
                        pred = row[bx - 1]
                    else:
                        pred = (top[bx] + row[bx - 1] + 1) >> 1
                    bctx = blk_map[(c ^ 1 if c < 2 else 2) * J.NUM_ORDERS]
                    nzv = min(pred, 64)
                    nzc = (nzv if nzv < 8 else 4 + nzv // 2) * nctx_blk + bctx
                    nzeros = sum(1 for k in range(1, 64) if q[order[k]])
                    toks.append((nzc, nzeros))
                    nz[c][by][bx] = nzeros
                    histo = nctx_blk * J.NONZERO_BUCKETS + J.ZERO_DENSITY_CONTEXTS * bctx
                    prev = 0 if nzeros > 64 // 16 else 1
                    k = 1
                    while k < 64 and nzeros:
                        ctx = histo + (J.COEFF_NZ_CTX[nzeros] + J.COEFF_FREQ_CTX[k]) * 2 + prev
                        u = pack_signed(int(q[order[k]]))
                        toks.append((ctx, u))
                        prev = 1 if u else 0
                        nzeros -= prev
                        k += 1
        return toks
    gtoks = [group_tokens(g) for g in range(ng)]
    ach = Histos(num_ac_ctx, ac_cluster_map(nctx_blk))
    for toks in gtoks:
        for ctx, v in toks:
            ach.add(ctx, hybrid_token(v)[0])
    ach.finish()

    small = w <= GROUP_DIM and h <= GROUP_DIM
    if alpha:                                   # alpha: extra channel, global tree with one leaf
        arows = [list(rgba[3 + 4 * y * w:3 + 4 * (y + 1) * w:4]) for y in range(h)]
        anodes, _ = tree_nodes(1)
        ahs = Histos(1)
        agroups = []
        for gy in range(yg):
            for gx in range(xg):
                sub = [row[gx * GROUP_DIM:(gx + 1) * GROUP_DIM] for row in arows[gy * GROUP_DIM:(gy + 1) * GROUP_DIM]]
                rs = residuals(sub)
                agroups.append(rs)
                for v in rs:
                    ahs.add(0, hybrid_token(pack_signed(v))[0])
        ahs.finish()

    gl = BW()                                   # ---- DC global
    gl.bool(True)                               # default DC quantization
    gl.u32(gs, (B(11, 1), B(11, 2049), B(12, 4097), B(16, 8193)))
    gl.u32(qdc, (V(16), B(5, 1), B(8, 1), B(16, 1)))
    gl.f(0, 1)                                  # custom block context map: no thresholds,
    gl.f(0, 12)                                 # 3 x 4 bits DC, (4 bits quant field below)
    gl.f(0, 4)
    gl.f(1, 1)                                  # simple context map, 2 bits per entry
    gl.f(2, 2)
    for v in blk_map:
        gl.f(v, 2)
    gl.f(1, 1)                                  # default colour correlation
    if alpha:
        gl.f(1, 1)                              # global tree (alpha)
        write_tree(gl, anodes)
        ahs.write(gl)
        group_header(gl, True, False)
        if small:
            st = ahs.stream(gl)
            for v in agroups[0]:
                st.put(0, pack_signed(v))
            st.flush()
    else:
        gl.f(0, 1)                              # no global tree (no modular channels)

    def dc_group(bw, g):
        gx, gy = g % xdg, g // xdg
        x0, y0 = gx * GROUP_DIM, gy * GROUP_DIM
        gw, gh = min(GROUP_DIM, xb - x0), min(GROUP_DIM, yb - y0)
        bw.f(0, 2)                              # extra DC precision 0
        chans = [[[int(v) for v in dcq_img[c][y0 + y, x0:x0 + gw]] for y in range(gh)] for c in (1, 0, 2)]
        write_modular_local(bw, chans)
        cnt = gw * gh                           # AC metadata: one DCT8 per block
        nb = J.ceil_log2(cnt)
        bw.f(cnt - 1, nb)
        cw, chh = (gw + 7) >> 3, (gh + 7) >> 3
        meta = [[[0] * cw for _ in range(chh)], [[0] * cw for _ in range(chh)],
                [[0] * cnt, [qf - 1] * cnt], [[0] * gw for _ in range(gh)]]
        write_modular_local(bw, meta)

    def ac_global(bw):
        bw.f(1, 1)                              # library quantization tables
        nb = J.ceil_log2(ng)
        if nb:
            bw.f(0, nb)                         # one histogram set
        bw.u32(0, (V(0x5F), V(0x13), V(0), B(J.NUM_ORDERS)))   # natural orders
        ach.write(bw)

    def ac_group(bw, g):
        st = ach.stream(bw)
        for ctx, v in gtoks[g]:
            st.put(ctx, v)
        st.flush()
        if alpha and not small:
            group_header(bw, True, False)
            st = ahs.stream(bw)
            for v in agroups[g]:
                st.put(0, pack_signed(v))
            st.flush()

    if ng == 1:
        dc_group(gl, 0)
        ac_global(gl)
        ac_group(gl, 0)
        gl.pad()
        sections = [gl.data()]
    else:
        gl.pad()
        sections = [gl.data()]
        for g in range(ndg):
            b = BW()
            dc_group(b, g)
            b.pad()
            sections.append(b.data())
        b = BW()
        ac_global(b)
        b.pad()
        sections.append(b.data())
        for g in range(ng):
            b = BW()
            ac_group(b, g)
            b.pad()
            sections.append(b.data())

    out = BW()
    out.f(0xFF, 8)
    out.f(0x0A, 8)
    size_header(out, w, h)
    if not alpha:
        out.bool(True)                          # all_default metadata: 8-bit sRGB, XYB
    else:
        out.bool(False)
        out.bool(False)
        out.bool(False)
        out.u32(8, (V(8), V(10), V(12), B(6, 1)))
        out.bool(True)
        out.u32(1, (V(0), V(1), B(4, 2), B(12, 1)))
        out.bool(True)                          # alpha all_default
        out.bool(True)                          # xyb_encoded
        out.bool(True)                          # sRGB
        out.u64(0)
    out.bool(True)                              # default transform data
    out.pad()
    out.bool(False)                             # frame header: VarDCT, XYB, qm scales 3 / 2
    out.u32(0, (V(0), V(1), V(2), V(3)))
    out.bool(False)
    out.u64(0)
    out.u32(1, (V(1), V(2), V(4), V(8)))
    if alpha:
        out.u32(1, (V(1), V(2), V(4), V(8)))
    out.f(3, 3)
    out.f(2, 3)
    out.u32(1, (V(1), V(2), V(3), B(3, 4)))
    out.bool(False)
    for _ in range(1 + (1 if alpha else 0)):
        out.u32(0, (V(0), V(1), V(2), B(2, 3)))
    out.bool(True)
    out.u32(0, (V(0), B(4), B(5, 16), B(10, 48)))
    out.bool(False)                             # loop filter: no gaborish, no EPF
    out.bool(False)
    out.f(0, 2)
    out.u64(0)
    out.u64(0)
    out.f(0, 1)                                 # TOC
    out.pad()
    for sct in sections:
        out.u32(len(sct), (B(10), B(14, 1024), B(22, 17408), B(30, 4211712)))
    out.pad()
    return out.data() + b"".join(sections)


# ====================================================================== container

def wrap_container(cs, exif=None, xmp=None):
    """the codestream in the container (jxl_enc.s je_container): signature box, ftyp jxl, Exif box
    (TIFF header offset 0, the EXIF as given), xml box, jxlc box"""
    if not exif and not xmp:
        return cs
    box = lambda t, b: struct.pack(">I", 8 + len(b)) + t + b
    out = b"\x00\x00\x00\x0cJXL \r\n\x87\n" + box(b"ftyp", b"jxl \x00\x00\x00\x00jxl ")
    if exif:
        out += box(b"Exif", b"\x00\x00\x00\x00" + exif)
    if xmp:
        out += box(b"xml ", xmp)
    return out + box(b"jxlc", cs)


# ====================================================================== PNG input (tests)

def read_png(path):
    """8-bit RGB / RGBA / gray / gray-alpha PNG without interlacing -> (w, h, RGBA bytes)"""
    data = Path(path).read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    p = 8
    idat = b""
    while p < len(data):
        n = struct.unpack(">I", data[p:p + 4])[0]
        kind = data[p + 4:p + 8]
        body = data[p + 8:p + 8 + n]
        if kind == b"IHDR":
            w, h, bd, ct, _, _, il = struct.unpack(">IIBBBBB", body)
            assert bd == 8 and il == 0 and ct in (0, 2, 4, 6), (bd, ct, il)
        elif kind == b"IDAT":
            idat += body
        p += 12 + n
    raw = zlib.decompress(idat)
    bpp = {0: 1, 2: 3, 4: 2, 6: 4}[ct]
    stride = w * bpp
    out = bytearray()
    prev = bytearray(stride)
    pos = 0
    for y in range(h):
        ft = raw[pos]
        line = bytearray(raw[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if ft == 1:
                line[i] = (line[i] + a) & 255
            elif ft == 2:
                line[i] = (line[i] + b) & 255
            elif ft == 3:
                line[i] = (line[i] + ((a + b) >> 1)) & 255
            elif ft == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        prev = line
        for x in range(w):
            q = line[x * bpp:(x + 1) * bpp]
            if ct == 0:
                out += bytes((q[0], q[0], q[0], 255))
            elif ct == 2:
                out += bytes((q[0], q[1], q[2], 255))
            elif ct == 4:
                out += bytes((q[0], q[0], q[0], q[1]))
            else:
                out += bytes(q)
    return w, h, bytes(out)


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(2)
    w, h, rgba = read_png(sys.argv[1])
    data = encode_lossless(w, h, rgba)
    Path(sys.argv[2]).write_bytes(data)
    print("%dx%d -> %d bytes" % (w, h, len(data)))


if __name__ == "__main__":
    main()
