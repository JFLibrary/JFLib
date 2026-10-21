#!/usr/bin/env python3
"""Checks of the AVIF reader (ASMLIB/src/img/av1*.s, heic.s, avif.s, isobmff.s, yuv.s) through
apps/codectest.

Suites
  av1      AV1 still streams (libaom, SVT-AV1 and rav1e through ffmpeg: 4:0:0 / 4:2:0 / 4:2:2 /
           4:4:4, 8 / 10 / 12 bits, odd sizes, palette / screen content, lossless, 128x128
           superblocks, tiles, quantizer matrices, segmentation, delta q, superres, loop
           restoration, film grain, reduced and full sequence headers) -> op 140 against libdav1d's
           planes (ffmpeg), bit-exact
  stages   the same op on a few streams after every in-loop stage (reconstruction, deblocking,
           CDEF, superres, loop restoration) against tools/av1_ref.py (slow: the model is Python)
  avif     AVIF files written here around such streams -> op 240 (img_decode_ex) against the YUV
           model of tools/heic_check.py, bit-exact: colour from colr nclx / the sequence header /
           nothing, chroma sample position, 4:0:0 / 4:4:4 / 10 bits, alpha (monochrome AV1 alpha
           item, prem), grids of AV1 tiles (clipped), clap / irot / imir, a file muxed by ffmpeg
  errors   malformed files (no av1C, short av1C, OBUs beyond the item, an inter frame) -> errors
  fuzz     truncated and mutated AVIF files and OBU streams: any result, no crash (both targets)
  aenc     the AV1 encoder (av1_enc.s, op 141) against tools/av1_enc_ref.py, byte for byte: 4:0:0 /
           4:2:0 / 4:4:4, 8 / 10 bits, lossless, odd sizes, base_q_idx 1 .. 255, several tiles; the
           model's streams decoded by libdav1d and tools/av1_ref.py (equal to its reconstruction)
  enc      the AVIF writer (avif_enc.s, img_encode IMGF_AVIF) -> read back (op 240): PSNR, lossless
           exactly, alpha, 4:0:0 / 4:4:4, metadata round trip; ffmpeg (libdav1d) and WIC (Windows' AV1
           extension, when installed) read the files
  perf     decode time of a 4032 x 3024 AVIF (op 161, SVT-AV1 encode) and of a grid, encode time

  python tools/avif_check.py --target both --out build/ac
  python tools/avif_check.py --target win --only av1,avif --no-build

Chroma position (heic.s): CSP_UNKNOWN -> centred (chroma location type 1), CSP_VERTICAL -> 0,
CSP_COLOCATED -> 2.  Streams and libdav1d's decodes are cached in <out>/avif_cache.
"""

import hashlib
import random
import struct
import sys
import tempfile
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
import heic_check as hc  # noqa: E402
import av1_ref  # noqa: E402

C = it.C
Rec = it.Rec
OP_AV1DEC, OP_AV1ENC, OP_HEICTIME = 140, 141, 161
OP_ENCMETA = 260                     # img_encode with an IMGMETA (png_ops.s)
STRAIGHT = C.IMGD_STRAIGHT
ROOT = Path(__file__).resolve().parent.parent
PHOTO = ROOT / "apps" / "PaintCraft" / "tests" / "fixtures" / "a6_photo.png"
URN_ALPHA = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha"
LOC = {0: 1, 1: 0, 2: 2}            # chroma_sample_position -> chroma location type (yuv.s)


# ================================================================== streams

def obus(data):
    """[(type, whole OBU bytes)] of a low-overhead stream"""
    out, p = [], 0
    while p < len(data):
        hdr = data[p]
        typ, ext, has = (hdr >> 3) & 15, (hdr >> 2) & 1, (hdr >> 1) & 1
        q = p + 1 + ext
        if has:
            sz, sh = 0, 0
            while True:
                b = data[q]
                q += 1
                sz |= (b & 0x7F) << sh
                sh += 7
                if not b & 0x80:
                    break
        else:
            sz = len(data) - q
        out.append((typ, data[p:q + sz]))
        p = q + sz
    return out


class AV1Stream:
    def __init__(self, obu, planes, w, h, chroma, bits):
        self.obu, self.planes, self.w, self.h, self.chroma, self.bits = obu, planes, w, h, chroma, bits
        parts = obus(obu)
        self.item = b"".join(o for t, o in parts if t != 2)          # without temporal delimiters
        self.seqobu = next(o for t, o in parts if t == 1)
        d = av1_ref.Decoder()
        b = av1_ref.Bits(self.seqobu)
        hdr = self.seqobu[0]
        q = 1 + ((hdr >> 2) & 1)
        while self.seqobu[q] & 0x80:
            q += 1
        b.p = (q + 1) * 8
        d.sequence_header_obu(b)
        self.seq = d

    def cicp(self):
        s = self.seq
        col = (s.color_primaries, s.transfer_characteristics, s.matrix_coefficients) \
            if getattr(s, "color_primaries", 2) is not None else (2, 2, 2)
        return col[0] | col[1] << 8 | col[2] << 16 | (s.color_range & 1) << 24

    def loc(self):
        return LOC.get(self.seq.chroma_sample_position, 1)

    def av1c(self):
        s = self.seq
        hbd = 1 if s.BitDepth > 8 else 0
        twelve = 1 if s.BitDepth == 12 else 0
        b2 = (hbd << 6) | (twelve << 5) | (s.mono_chrome << 4) | (s.subsampling_x << 3) | \
            (s.subsampling_y << 2) | (s.chroma_sample_position & 3)
        return hc.box(b"av1C", bytes([0x81, (s.seq_profile << 5) | (s.seq_level_idx & 31), b2, 0]) + self.seqobu)


def encode_args(enc, params):
    if enc == "aom":
        return ["-c:v", "libaom-av1", "-still-picture", "1"] + params.split()
    if enc == "svt":
        return ["-c:v", "libsvtav1"] + params.split()
    return ["-c:v", "librav1e"] + params.split()


def make_av1(cache, w, h, chroma, bits, enc="aom", params="-crf 30 -cpu-used 6", seed=1, kind="photo",
             planes=None):
    """Encode planes (or generated content) as one AV1 frame; cached with libdav1d's decode."""
    key = hashlib.sha1(repr((w, h, chroma, bits, enc, params, seed, kind,
                             None if planes is None else [hashlib.sha1(p.tobytes()).hexdigest()
                                                         for p in planes])).encode()).hexdigest()[:16]
    cache.mkdir(parents=True, exist_ok=True)
    fs, fy = cache / f"{key}.obu", cache / f"{key}.yuv"
    pf = hc.PIXFMT[(chroma, bits)]
    if not (fs.exists() and fy.exists()):
        if planes is None:
            planes = hc.gen_planes(seed, w, h, chroma, bits, kind) if kind != "file" else None
        with tempfile.TemporaryDirectory() as td:
            if kind == "file":
                inp = ["-i", str(PHOTO), "-vf", f"scale={w}:{h}"]
            else:
                ri = Path(td) / "in.raw"
                ri.write_bytes(b"".join((p.astype(np.uint8) if bits == 8 else p.astype("<u2")).tobytes()
                                        for p in planes))
                inp = ["-f", "rawvideo", "-pix_fmt", pf, "-s", f"{w}x{h}", "-i", str(ri)]
            hc.run_ff(inp + ["-frames:v", "1", "-pix_fmt", pf] + encode_args(enc, params) +
                      ["-f", "obu", str(fs)])
        hc.run_ff(["-c:v", "libdav1d", "-i", str(fs), "-f", "rawvideo", "-pix_fmt", pf, str(fy)])
    yuv = fy.read_bytes()
    dt = np.uint8 if bits == 8 else np.dtype("<u2")
    a = np.frombuffer(yuv, dt).astype(np.uint16)
    sw, sh = hc.SUB[chroma]
    out, p = [], 0
    for c in range(1 if chroma == 0 else 3):
        pw, ph = (w, h) if c == 0 else (-(-w // sw), -(-h // sh))
        out.append(a[p:p + pw * ph].reshape(ph, pw))
        p += pw * ph
    return AV1Stream(fs.read_bytes(), out, w, h, chroma, bits)


# ================================================================== suites: streams

SPECS = [  # (encoder, w, h, chroma, bits, kind, params)
    ("aom", 64, 64, 1, 8, "photo", "-crf 30 -cpu-used 6"),
    ("aom", 256, 192, 1, 8, "file", "-crf 20 -cpu-used 4"),
    ("aom", 97, 61, 1, 8, "photo", "-crf 28 -cpu-used 5"),
    ("aom", 96, 80, 3, 8, "photo", "-crf 25 -cpu-used 4"),
    ("aom", 128, 72, 1, 10, "photo", "-crf 28 -cpu-used 5"),
    ("aom", 96, 64, 1, 12, "file", "-crf 30 -cpu-used 5"),
    ("aom", 96, 64, 2, 10, "file", "-crf 30 -cpu-used 5"),
    ("aom", 80, 48, 0, 8, "photo", "-crf 30 -cpu-used 5"),
    ("aom", 64, 48, 0, 10, "photo", "-crf 30 -cpu-used 5"),
    ("aom", 100, 60, 1, 8, "photo", "-crf 0 -lossless 1 -cpu-used 5"),
    ("aom", 48, 40, 3, 10, "photo", "-crf 0 -lossless 1 -cpu-used 5"),
    ("aom", 128, 128, 3, 8, "photo", "-crf 30 -cpu-used 5 -aom-params tune-content=screen"),
    ("aom", 200, 150, 1, 8, "file", "-crf 35 -cpu-used 2 -aom-params enable-restoration=1:sb-size=128"),
    ("aom", 256, 128, 1, 8, "file", "-crf 30 -cpu-used 5 -tile-columns 1 -tile-rows 1 -aom-params "
     "enable-qm=1:qm-min=2:qm-max=10"),
    ("aom", 128, 96, 1, 8, "file", "-crf 30 -cpu-used 5 -aom-params film-grain-test=1"),
    ("aom", 160, 120, 1, 8, "file", "-crf 24 -cpu-used 3"),
    ("aom", 160, 96, 1, 8, "file", "-crf 30 -usage allintra -cpu-used 6"),
    ("aom", 90, 70, 3, 12, "file", "-crf 20 -cpu-used 5"),
    ("aom", 33, 17, 1, 8, "photo", "-crf 40 -cpu-used 6"),
    ("svt", 192, 128, 1, 8, "file", "-crf 35 -preset 8 -svtav1-params "
     "superres-mode=1:superres-denom=12:superres-kf-denom=12"),
    ("svt", 192, 128, 1, 8, "file", "-crf 35 -preset 6 -svtav1-params aq-mode=1"),
    ("svt", 160, 128, 1, 10, "file", "-crf 30 -preset 6"),
    ("rav1e", 160, 96, 1, 8, "file", "-qp 120 -speed 6"),
    ("rav1e", 160, 96, 1, 8, "file", "-qp 80 -speed 4 -rav1e-params tune=Psychovisual"),
]


def av1_streams(cache):
    out = []
    for i, (enc, w, h, ch, bd, kind, prm) in enumerate(SPECS):
        try:
            out.append((f"{enc} {w}x{h} ch{ch} bd{bd} {prm}", make_av1(cache, w, h, ch, bd, enc, prm, seed=i + 1,
                                                                     kind=kind)))
        except RuntimeError as e:
            print(f"  (skipped {enc} {w}x{h} {prm}: {str(e)[:120]})")
    return out


def planes_check(st):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        out = bytes(out)
        w, h, bd, sx, sy, mono = struct.unpack_from("<6I", out, 0)
        got = np.frombuffer(out, np.uint16, offset=24)
        ref = np.concatenate([p.ravel() for p in st.planes])
        if (w, h, bd) != (st.w, st.h, st.bits) or len(got) != len(ref):
            return f"geometry {w}x{h} bd{bd} n{len(got)} vs {st.w}x{st.h} bd{st.bits} n{len(ref)}"
        d = np.nonzero(got != ref)[0]
        return None if len(d) == 0 else f"{len(d)} samples differ (first at {d[0]})"
    return chk


def suite_av1(runners, streams, suites):
    recs = [Rec(OP_AV1DEC, 0, st.obu, planes_check(st), name) for name, st in streams]
    for t, r in runners.items():
        s = it.Suite("av1", t)
        it.run_suite(r, s, recs)
        suites.append(s)


def suite_stages(runners, streams, suites):
    stages = {"recon": 1, "lf": 2, "cdef": 3, "sr": 4, "lr": 5}
    pick = [(n, st) for n, st in streams if st.w * st.h <= 160 * 128][:8]
    recs = []
    for name, st in pick:
        for sn, sv in stages.items():
            fr = av1_ref.Decoder(True, sn).decode(st.obu)[0]
            ref = AV1Stream.__new__(AV1Stream)
            ref.w, ref.h, ref.bits = fr.w, fr.h, fr.bitdepth
            ref.planes = [np.array(p, np.uint16) for p in fr.planes]
            recs.append(Rec(OP_AV1DEC, sv | 0x100, st.obu, planes_check(ref), f"{name} [{sn}]"))
    for t, r in runners.items():
        s = it.Suite("stages", t)
        it.run_suite(r, s, recs)
        suites.append(s)


# ================================================================== suites: AVIF files

def single(st, h=None, nclx_box=None, extra_props=(), o=None, split=1):
    h = h or hc.Heif()
    iid = h.item(b"av01", st.item, split=split)
    h.prop(st.av1c(), iid, True)
    h.prop(hc.ispe(st.w, st.h), iid)
    if nclx_box:
        h.prop(nclx_box, iid)
    for p in extra_props:
        h.prop(p, iid, True)
    return h, iid


def build(h, iid, o=None):
    o = dict(o or {})
    o.setdefault("ftyp_brands", (b"avif", [b"mif1", b"miaf", b"avif"]))
    return h.build(iid, o)


def model(st, cicp=None):
    return hc.yuv_to_bgra(st.planes, st.chroma, st.bits, st.bits, st.cicp() if cicp is None else cicp, st.loc())


def colr_in(data):
    """CICP of the first colr nclx in a file (for ffmpeg's files), or None"""
    p = data.find(b"colrnclx")
    if p < 0:
        return None
    b = data[p + 8:p + 15]
    pr, tr, mc = struct.unpack(">HHH", b[:6])
    return min(pr, 255) | min(tr, 255) << 8 | min(mc, 255) << 16 | (b[6] >> 7) << 24


def build_cases(cache, seed):
    cases, S = [], {}

    def st(name, *a, **k):
        if name not in S:
            S[name] = make_av1(cache, *a, **k)
        return S[name]
    Case = hc.Case
    base = st("base", 96, 64, 1, 8, "aom", "-crf 28 -cpu-used 5", seed=3)
    # colour: no colr (sequence header), nclx variants
    h, iid = single(base)
    cases.append(Case("sequence header colour", build(h, iid), model(base), meta={"w": 96, "h": 64}))
    for (p, t, m, full) in [(1, 13, 6, 1), (1, 1, 1, 0), (9, 16, 9, 0), (2, 2, 2, 0), (1, 13, 0, 1),
                            (12, 13, 12, 1), (2, 2, 8, 1)]:
        cicp = p | t << 8 | m << 16 | full << 24
        h, iid = single(base, nclx_box=hc.nclx(p, t, m, full))
        cases.append(Case(f"nclx {p}/{t}/{m}/{full}", build(h, iid), model(base, cicp),
                          meta={"w": 96, "h": 64, "cicp": cicp}))
    col = st("col", 80, 48, 1, 8, "aom", "-crf 30 -cpu-used 6 -color_primaries bt709 -color_trc bt709 "
             "-colorspace bt709 -color_range pc", seed=4)
    h, iid = single(col)
    cases.append(Case("colour description in the stream", build(h, iid), model(col)))
    for name, ch, bd, prm in [("444", 3, 8, "-crf 25 -cpu-used 5"), ("10bit", 1, 10, "-crf 30 -cpu-used 6"),
                              ("mono", 0, 8, "-crf 30 -cpu-used 6"), ("422 12bit", 2, 12, "-crf 30 -cpu-used 6"),
                              ("lossless 444", 3, 8, "-crf 0 -lossless 1 -cpu-used 6")]:
        s = st(name, 72, 40, ch, bd, "aom", prm, seed=10 + bd + ch)
        h, iid = single(s)
        cases.append(Case(f"format {name}", build(h, iid), model(s), meta={"bits": bd}))
    odd = st("odd", 37, 23, 1, 8, "aom", "-crf 30 -cpu-used 6", seed=5)
    h, iid = single(odd)
    cases.append(Case("odd size 37x23", build(h, iid), model(odd)))
    # transformations
    exp0 = model(base)
    for props in [[("irot", 1)], [("imir", 0)], [("imir", 1)], [("irot", 3), ("imir", 1)],
                  [("clap", (64, 1, 40, 1, 0, 1, 0, 1))], [("clap", (50, 1, 30, 1, -5, 1, 3, 1)), ("irot", 1)]]:
        rect, o = hc.apply_props(exp0, props)
        x0, y0, ww, hh = rect
        exp = exp0[y0:y0 + hh, x0:x0 + ww]
        exp = it.orient(exp, o)
        h, iid = single(base, extra_props=hc.prop_boxes(props))
        cases.append(Case(f"props {props}", build(h, iid), exp))
    # alpha
    a8 = st("alpha8", 96, 64, 0, 8, "aom", "-crf 20 -cpu-used 6", seed=7, kind="alpha")
    for prem in (False, True):
        h, cid = single(base)
        aid = h.item(b"av01", a8.item)
        h.prop(a8.av1c(), aid, True)
        h.prop(hc.ispe(96, 64), aid)
        h.prop(hc.fbox(b"auxC", 0, 0, URN_ALPHA + b"\0"), aid, True)
        h.ref(b"auxl", aid, [cid])
        exp = model(base).copy()
        exp[..., 3] = hc.alpha8(a8.planes[0], a8.bits)
        flags, fmt, mf = STRAIGHT, C.FMT_ARGB, C.IMF_ALPHA
        if prem:
            h.ref(b"prem", cid, [aid])
            exp[..., :3] = np.minimum(exp[..., :3], exp[..., 3:4])
            flags, fmt = 0, C.FMT_PARGB
        cases.append(Case(f"alpha prem={prem}", build(h, cid), exp, flags=flags, fmt=fmt, meta={"channels": 4}))
    # grids
    for (w, hh_, tw, th, ch) in [(120, 90, 64, 48, 1), (200, 64, 64, 64, 3)]:
        cols, rows = -(-w // tw), -(-hh_ // th)
        src = hc.gen_planes(seed + w, cols * tw, rows * th, ch, 8)
        sw, sh = hc.SUB[ch]
        h = hc.Heif()
        gid = h.item(b"grid", bytes([0, 0, rows - 1, cols - 1]) + struct.pack(">HH", w, hh_))
        tiles = []
        exp = np.zeros((hh_, w, 4), np.uint8)
        for r in range(rows):
            for c in range(cols):
                tp = [src[0][r * th:(r + 1) * th, c * tw:(c + 1) * tw]] + \
                     [p[r * th // sh:(r + 1) * th // sh, c * tw // sw:(c + 1) * tw // sw] for p in src[1:]]
                ts = make_av1(cache, tw, th, ch, 8, "aom", "-crf 28 -cpu-used 6", planes=tp)
                tid = h.item(b"av01", ts.item, hidden=True)
                h.prop(ts.av1c(), tid, True)
                h.prop(hc.ispe(tw, th), tid)
                tiles.append(tid)
                rgb = model(ts)
                y1, x1 = min(hh_, (r + 1) * th), min(w, (c + 1) * tw)
                exp[r * th:y1, c * tw:x1] = rgb[:y1 - r * th, :x1 - c * tw]
        h.ref(b"dimg", gid, tiles)
        h.prop(hc.ispe(w, hh_), gid)
        cases.append(Case(f"grid {cols}x{rows} {tw}x{th} -> {w}x{hh_} ch{ch}", build(h, gid), exp,
                          meta={"w": w, "h": hh_}))
    # a file muxed by ffmpeg
    with tempfile.TemporaryDirectory() as td:
        f = Path(td) / "x.avif"
        src = Path(td) / "x.obu"
        src.write_bytes(base.obu)
        try:
            hc.run_ff(["-i", str(src), "-c", "copy", "-f", "avif", str(f)])
            data = f.read_bytes()
            cicp = colr_in(data)
            cases.append(Case("ffmpeg avif muxer", data, model(base, cicp)))
        except RuntimeError as e:
            print("  (no ffmpeg avif muxer: %s)" % str(e)[:80])
    # container variants
    for v in [dict(iloc_v=0), dict(iloc_v=2), dict(idat=1), dict(mdat_first=True), dict(large=True)]:
        h, iid = single(base)
        if v.get("idat"):
            h.items[0]["cm"] = 1
            v = {}
        cases.append(Case(f"container {v or 'idat'}", build(h, iid, v), model(base)))
    h, iid = single(base, split=4)
    cases.append(Case("4 extents", build(h, iid), model(base)))
    return cases, S


def error_cases(S):
    base = S["base"]
    Case = hc.Case
    out = []
    h = hc.Heif()
    iid = h.item(b"av01", base.item)
    h.prop(hc.ispe(96, 64), iid)
    out.append(Case("no av1C", build(h, iid), err=C.ERR_HDR))
    h, iid = single(base)
    h.props[0] = hc.box(b"av1C", b"\x81\0")
    out.append(Case("short av1C", build(h, iid), err=C.ERR_HDR))
    h, iid = single(base)
    h.items[0]["data"] = base.item[:len(base.item) // 2]
    out.append(Case("item cut in the middle of an OBU", build(h, iid), err=(C.ERR_TRUNC, C.ERR_HDR)))
    h, iid = single(base)
    h.items[0]["data"] = base.seqobu
    out.append(Case("sequence header only", build(h, iid), err=C.ERR_TRUNC))
    return out


def suite_cases(runners, cases, suites, name):
    recs = [c.rec() for c in cases]
    for t, r in runners.items():
        s = it.Suite(name, t)
        it.run_suite(r, s, recs)
        suites.append(s)


def suite_fuzz(runners, cases, streams, suites, n, seed):
    recs = hc.mutants(cases, n // 2, seed)
    rng = random.Random(seed + 1)
    src = [st.obu for _, st in streams if len(st.obu) < 20000]
    for i in range(n - n // 2):
        d = bytearray(rng.choice(src))
        k = rng.random()
        if k < 0.3:
            d = d[:rng.randrange(len(d))]
        else:
            for _ in range(rng.randrange(1, 8)):
                p = rng.randrange(len(d))
                d[p] = rng.randrange(256) if k < 0.7 else d[p] ^ (1 << rng.randrange(8))
        recs.append(Rec(OP_AV1DEC, 0, bytes(d), it.any_result, f"obu mut{i}"))
    for t, r in runners.items():
        s = it.Suite("fuzz", t)
        it.run_suite(r, s, recs)
        suites.append(s)


# ================================================================== suites: encoder

def aenc_payload(planes, w, h, chroma, bd, qidx, tile_sb=4, cicp=1 | 13 << 8 | 1 << 16 | 1 << 24):
    return struct.pack("<8I", w, h, bd, chroma, qidx, cicp, tile_sb, 0) + \
        b"".join(np.asarray(p, dtype="<u2").tobytes() for p in planes)


def aenc_photo(w, h, chroma, bits):
    """photo crop as planes (u16 arrays), 10 bits by replication"""
    pf = {0: "gray", 1: "yuv420p", 3: "yuv444p"}[chroma]
    raw = hc.run_ff(["-i", str(PHOTO), "-vf", f"crop=iw/3:ih/3:iw/4:ih/5,scale={w}:{h}:flags=neighbor",
                     "-pix_fmt", pf, "-f", "rawvideo", "-"])
    a = np.frombuffer(raw, np.uint8).astype(np.uint16)
    sw, sh = hc.SUB[chroma]
    out, p = [], 0
    for c in range(1 if chroma == 0 else 3):
        pw, ph = (w, h) if c == 0 else (-(-w // sw), -(-h // sh))
        pl = a[p:p + pw * ph].reshape(ph, pw)
        out.append(np.minimum(1023, (pl << 2) | (pl >> 6)) if bits == 10 else pl)
        p += pw * ph
    return out


AENC = [  # (w, h, chroma, bits, base_q_idx, tile_sb, content)
    (8, 8, 0, 8, 100, 4, "gen"), (40, 24, 1, 8, 100, 4, "gen"), (72, 40, 1, 8, 60, 4, "gen"),
    (72, 40, 3, 8, 120, 4, "gen"), (72, 40, 0, 8, 80, 4, "gen"), (72, 40, 1, 10, 100, 4, "gen"),
    (24, 16, 3, 8, 0, 4, "gen"), (24, 16, 1, 8, 0, 4, "gen"), (200, 136, 1, 8, 140, 1, "gen"),
    (1, 1, 1, 8, 100, 4, "photo"), (3, 5, 0, 8, 60, 4, "photo"), (17, 9, 1, 8, 1, 4, "photo"),
    (33, 65, 3, 8, 20, 4, "photo"), (65, 33, 1, 10, 255, 4, "photo"), (70, 70, 1, 8, 1, 4, "photo"),
    (96, 72, 3, 10, 40, 4, "photo"), (130, 70, 1, 8, 128, 1, "photo"), (64, 64, 0, 10, 0, 4, "photo"),
    (41, 23, 1, 8, 0, 4, "photo"),
]


def aenc_case(cache, i, spec, no_ext):
    """(payload, expected stream) of an AENC case; the model's stream is cached and checked once
    against libdav1d and tools/av1_ref.py"""
    import av1_enc_ref as E
    w, h, ch, bd, q, tsb, kind = spec
    planes = hc.gen_planes(100 + i, w, h, ch, bd) if kind == "gen" else aenc_photo(w, h, ch, bd)
    pay = aenc_payload(planes, w, h, ch, bd, q, tsb)
    key = hashlib.sha1(pay + b"aenc1").hexdigest()[:16]
    cache.mkdir(parents=True, exist_ok=True)
    f = cache / f"aenc_{key}.obu"
    if f.exists():
        return pay, f.read_bytes(), None
    data, e = E.encode([p.astype(int).tolist() for p in planes], w, h, ch, bd, q, True, tsb)
    rec = av1_ref.Decoder(True, "recon").decode(data)[0]
    for p_, pl in enumerate(rec.planes):
        for y, row in enumerate(pl):
            if row != e.recon[p_][y][:len(row)]:
                return pay, data, "model: av1_ref's reconstruction differs from the encoder's"
    if not no_ext:
        fin = av1_ref.Decoder(True).decode(data)[0]
        pf = hc.PIXFMT[(ch, bd)]
        dec = hc.run_ff(["-c:v", "libdav1d", "-f", "obu", "-i", "-", "-f", "rawvideo", "-pix_fmt", pf, "-"],
                        b"\x12\x00" + data)
        dt = np.uint8 if bd == 8 else np.dtype("<u2")
        exp = np.concatenate([np.asarray(pl, dt).ravel() for pl in fin.planes]).tobytes()
        if dec != exp:
            return pay, data, "model: libdav1d's decode differs from av1_ref"
    f.write_bytes(data)
    return pay, data, None


def suite_aenc(runners, cache, suites, no_ext):
    recs, pre = [], []
    for i, spec in enumerate(AENC):
        pay, exp, err = aenc_case(cache, i, spec, no_ext)
        name = "%dx%d c%d b%d q%d t%d %s" % spec
        if err:
            pre.append(f"{name}: {err}")

        def chk(res, out, exp=exp):
            if res != 0:
                return f"error {it.err_name(res)}"
            out = bytes(out)
            if out != exp:
                d = next((k for k in range(min(len(out), len(exp))) if out[k] != exp[k]), min(len(out), len(exp)))
                return f"differs from the model at byte {d} (asm {len(out)}, model {len(exp)})"
            return None
        recs.append(Rec(OP_AV1ENC, 0, pay, chk, name))
    for t, R in runners.items():
        st = it.Suite("aenc", t)
        for m in pre:
            st.fail(m)
        it.run_suite(R, st, recs)
        suites.append(st)


def avif_opt(q=0, flags=0, chroma=0):
    return struct.pack("<12IQQ", C.IMGF_AVIF, q, flags, 0, 0, 0, 0, chroma, 0, 0, 0, 0, 0, 0)


def suite_enc(runners, suites, seed, out, no_ext):
    rng = np.random.default_rng(seed)
    cases = [  # name, w, h, quality, lossless, chroma, alpha, PSNR limit
        ("1x1", 1, 1, 90, 0, 0, False, 20), ("2x2 444", 2, 2, 90, 0, 444, False, 30),
        ("64x48", 64, 48, 90, 0, 0, False, 33), ("33x17 odd", 33, 17, 85, 0, 0, False, 32),
        ("600x350", 600, 350, 85, 0, 0, False, 33), ("120x90 444", 120, 90, 95, 0, 444, False, 38),
        ("120x90 422 as 420", 120, 90, 90, 0, 422, False, 33), ("120x90 400", 120, 90, 90, 0, 400, False, 0),
        ("96x64 alpha", 96, 64, 90, 0, 0, True, 33), ("700x300 alpha", 700, 300, 80, 0, 0, True, 32),
        ("q1", 64, 64, 1, 0, 0, False, 18), ("q100", 64, 64, 100, 0, 444, False, 42),
        ("default quality", 64, 64, 0, 0, 0, False, 33), ("lossless", 40, 30, 0, 1, 0, False, 99),
        ("lossless odd", 37, 21, 0, 1, 0, False, 99), ("lossless alpha", 300, 200, 0, 1, 0, True, 99)]
    recs, imgs = [], {}
    for name, w, h, q, ll, ch, alpha, lim in cases:
        img = hc.photo(rng, w, h, alpha)
        imgs[name] = img
        recs.append(Rec(it.OP_ENCODE, 0, avif_opt(q, ll, ch) + it.surface_input(img, C.FMT_ARGB),
                        lambda res, o: None if res == 0 else f"error {it.err_name(res)}", name))
    for t, R in runners.items():
        st = it.Suite("enc", t)
        res = it.run_suite(R, st, recs)
        files, drecs = {}, []
        for (name, w, h, q, ll, ch, alpha, lim), r in zip(cases, res):
            if r is None or r[0] != 0:
                continue
            files[name] = bytes(r[1])
            img = imgs[name]

            def dchk(res2, o, img=img, ll=ll, lim=lim, alpha=alpha, ch=ch):
                if res2 != 0:
                    return f"decode error {it.err_name(res2)}"
                d = it.parse_decex(o)
                if d["px"] is None or d["px"].shape != img.shape:
                    return f"size {None if d['px'] is None else d['px'].shape}"
                if ll:
                    m = it.max_abs(d["px"], img)
                    return None if m == 0 else f"lossless: max |d| {m}"
                if alpha and it.psnr(d["px"][..., 3], img[..., 3]) < 40:
                    return f"alpha PSNR {it.psnr(d['px'][..., 3], img[..., 3]):.1f}"
                if ch == 400:
                    px = d["px"][..., :3].astype(int)
                    if np.any(px[..., 0] != px[..., 1]) or np.any(px[..., 1] != px[..., 2]):
                        return "4:0:0 is not grey"
                    im = img.astype(int)
                    yv = (13933 * im[..., 2] + 46871 * im[..., 1] + 4732 * im[..., 0] + 32768) >> 16
                    q_ = it.psnr(px[..., 0], yv)
                    return None if q_ >= 33 else f"luma PSNR {q_:.1f}"
                q_ = it.psnr(d["px"][..., :3], img[..., :3])
                return None if q_ >= lim else f"PSNR {q_:.1f} < {lim}"
            drecs.append(Rec(it.OP_DECEX, 0, it.decex_input(files[name], flags=STRAIGHT), dchk, "dec " + name))
        it.run_suite(R, st, drecs)
        # metadata round trip
        import png_check as P
        icc = P.fake_icc(700, seed)
        exif = it.tiff_bytes([{"entries": [(0x0112, 3, [6]), (0x010F, 2, b"avif_check\0")], "next": None}])
        xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/">avif writer</x:xmpmeta>'
        img = hc.photo(rng, 80, 60)
        flags = C.IOF_EMBED_ICC | C.IOF_EMBED_EXIF | C.IOF_EMBED_XMP
        rr, enc = R.one(OP_ENCMETA, 0, P.encmeta_input(avif_opt(90, flags), img, C.FMT_ARGB, icc=icc, exif=exif,
                                                       xmp=xmp))
        if rr != 0:
            st.fail(f"meta encode: {it.err_name(rr)}")
        else:
            rr, o = R.one(it.OP_DECEX, 0, it.decex_input(bytes(enc), flags=STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF |
                                                         C.IMGD_XMP))
            d = it.parse_decex(o) if rr == 0 else None
            st.check(d is not None and d["icc"] == icc, "ICC round trip")
            st.check(d is not None and d["xmp"] == xmp, "XMP round trip")
            ok = d is not None and len(d["exif"]) == len(exif) and d["exif"] != exif and d["meta"]["orient"] == 1
            st.check(ok, "EXIF round trip (orientation set to 1)")
        # ffmpeg (libdav1d) reads the colour item of the files without alpha
        if not no_ext:
            work = out / "avif_enc_ext"
            work.mkdir(parents=True, exist_ok=True)
            got = 0
            for name, data in files.items():
                if "alpha" in name or "lossless" in name or name in ("1x1", "2x2 444"):
                    continue
                pth = work / (name.replace(" ", "_") + ".avif")
                pth.write_bytes(data)
                try:
                    dec = hc.run_ff(["-i", str(pth), "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
                except RuntimeError as e:
                    st.fail(f"ffmpeg cannot read {name}: {str(e)[:120]}")
                    continue
                img = imgs[name]
                if len(dec) != img.size:
                    st.fail(f"ffmpeg {name}: {len(dec)} bytes for {img.shape}")
                    continue
                px = np.frombuffer(dec, np.uint8).reshape(img.shape)
                if name.endswith("400"):
                    im = img.astype(int)
                    yv = (13933 * im[..., 2] + 46871 * im[..., 1] + 4732 * im[..., 0] + 32768) >> 16
                    q_ = it.psnr(px[..., 1], yv)
                else:
                    q_ = it.psnr(px[..., :3], img[..., :3])
                lim = 18 if name == "q1" else 28
                st.check(q_ >= lim, f"ffmpeg {name}: PSNR {q_:.1f} against the source")
                got += 1
            st.notes.append(f"ffmpeg read {got} written files")
        # WIC (Windows' AV1 extension): same pixels within the colour conversion / chroma filter
        if t == "win" and not no_ext:
            work = out / "avif_wic_enc"
            work.mkdir(parents=True, exist_ok=True)
            wic = it.Wic(work)
            ids = {}
            for name, data in files.items():
                if "lossless" in name:
                    continue
                pth = work / (name.replace(" ", "_") + ".avif")
                pth.write_bytes(data)
                ids[name] = wic.decode(pth)
            wic.run()
            got = 0
            for name, i in ids.items():
                r = wic.result(i)
                if r is None:
                    st.notes.append(f"WIC cannot read {name}")
                    continue
                got += 1
                px, img = r[3], imgs[name]
                if px.shape != img.shape:
                    st.fail(f"WIC {name}: size {px.shape} vs {img.shape}")
                    continue
                if name.endswith("400"):
                    continue
                q_ = it.psnr(px[..., :3], img[..., :3])
                lim = 18 if name == "q1" else 28
                st.check(q_ >= lim, f"WIC {name}: PSNR {q_:.1f} against the source")
                if "alpha" in name:
                    if np.all(px[..., 3] == 255):
                        st.notes.append(f"WIC {name}: no alpha (WIC ignores auxiliary alpha items)")
                    else:
                        st.check(it.psnr(px[..., 3], img[..., 3]) >= 35, f"WIC {name}: alpha differs")
            st.notes.append(f"WIC read {got} / {len(ids)} written files")
        suites.append(st)


def suite_perf(runners, cache, suites):
    big = make_av1(cache, 4032, 3024, 1, 8, "svt", "-crf 30 -preset 8", kind="file")
    h, iid = single(big)
    data = build(h, iid)
    exp = model(big)
    for t, R in runners.items():
        s = it.Suite("perf", t)
        res, out = R.one(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT))
        s.check(res == 0 and it.max_abs(it.parse_decex(out)["px"], exp) == 0, "12 MP AVIF: pixels differ")
        reps = 2
        res, out = R.one(OP_HEICTIME, reps | (STRAIGHT << 16), data)
        if res != 0:
            s.fail(f"perf: error {it.err_name(res)}")
        else:
            ms = struct.unpack("<Q", out)[0] / 1000.0 / reps
            s.notes.append(f"4032x3024 AVIF (SVT-AV1 crf 30): {ms:.0f} ms per decode ({len(data) // 1024} KB)")
        img = hc.photo(np.random.default_rng(5), 4032, 3024)
        t0 = time.time()
        res, enc = R.one(it.OP_ENCODE, 0, avif_opt(85) + it.surface_input(img, C.FMT_ARGB))
        dt = time.time() - t0
        s.check(res == 0, f"12 MP encode: {it.err_name(res)}")
        if res == 0:
            s.notes.append(f"4032x3024 AVIF encode (quality 85, 4:2:0): {dt * 1000:.0f} ms incl. process start, "
                           f"{len(enc) // 1024} KB")
        suites.append(s)


def main():
    ap = it.argparser(__doc__, "av1,stages,avif,errors,fuzz,aenc,enc,perf")
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(a.only.split(",")) if a.only else {"av1", "avif", "errors", "fuzz", "aenc", "enc", "perf"}
    runners = {t: it.Runner(t, out, work_name="avif") for t in targets}
    cache = out / "avif_cache"
    suites = []
    streams = av1_streams(cache) if only & {"av1", "stages", "fuzz"} else []
    if "av1" in only:
        suite_av1(runners, streams, suites)
    if "stages" in only:
        suite_stages(runners, streams, suites)
    cases, S = build_cases(cache, a.seed) if only & {"avif", "errors", "fuzz"} else ([], {})
    if "avif" in only:
        suite_cases(runners, cases, suites, "avif")
    if "errors" in only:
        suite_cases(runners, error_cases(S), suites, "errors")
    if "fuzz" in only:
        suite_fuzz(runners, cases, streams, suites, a.fuzz, a.seed)
    if "aenc" in only:
        suite_aenc(runners, cache, suites, a.no_ext)
    if "enc" in only:
        suite_enc(runners, suites, a.seed, out, a.no_ext)
    if "perf" in only:
        suite_perf(runners, cache, suites)
    ok = it.report(suites, "avif_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
