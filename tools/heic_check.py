#!/usr/bin/env python3
"""Checks of the HEIC reader (ASMLIB/src/img/heic.s, isobmff.s, yuv.s, hevc*.s) through
apps/codectest.

Suites
  hevc     raw HEVC streams (x265 through ffmpeg: 4:0:0 / 4:2:0 / 4:2:2 / 4:4:4, 8 / 10 / 12
           bits, odd sizes, slices, wavefront, transform skip, lossless, ...) -> op 160 against
           ffmpeg's decoded planes, bit-exact
  heif     HEIF files written by this script around such streams -> op 240 (img_decode_ex)
           against the Python model below, bit-exact: single images, grids (clipped output),
           alpha (both auxC URNs, alpha grids, prem), clap / irot / imir in every order, colr
           nclx (every matrix, full / limited range), VUI colour, no colour information,
           container variants (iloc v0 / v1 / v2 and field sizes, base offsets, several
           extents, idat, infe v3, pitm v1, ipma v1 / 16-bit indices, 64-bit box sizes, mdat
           before meta, NAL length sizes 1 / 2 / 4)
  meta     IMGMETA fields, IMGD_PROBE, ICC (colr prof), EXIF (Exif item, TIFF header offset),
           XMP (mime item), IMGD_NO_ORIENT
  errors   malformed files (no pitm, no hdlr pict, data outside the file, bad hvcC, missing
           tiles, unsupported item types, av01 without av1C) -> the expected error
  fuzz     truncated and mutated files: any result, no crash (both targets)
  henc     HEVC encoder (hevc_enc.s, op 162) byte-exact against tools/hevc_enc_ref.py: 4:0:0 /
           4:2:0 / 4:4:4, QP 0 .. 51, lossless, odd sizes, deblocking off; every stream decodes
           (ffmpeg) to the encoder's reconstruction
  enc      HEIC writer (heic_enc.s) through img_encode: sizes 1 x 1 .. 1100 x 600 (grids, odd
           sizes / clap), 4:2:0 / 4:4:4, quality 1 .. 100, alpha, lossless (exact, also alpha);
           our decoder: PSNR limits; metadata round trip (ICC / EXIF with orientation 1 / XMP);
           WIC reads every file to our pixels (+-, PSNR >= 30 dB)
  perf     decode time of a 4032 x 3024 grid of 512 x 512 tiles (iPhone layout; op 161), encode
           time of a 12 MP image
  wic      optional: Windows' HEIF decoder (WIC, needs the HEIF + HEVC extensions) on the
           same files, +-3 (colour conversion differs); skipped when WIC cannot decode HEIC

  python tools/heic_check.py --target both --out build/hc
  python tools/heic_check.py --target win --only heif --no-build

Model (yuv.s): range scaling Y' = (Y - yoff) * 255 / ymax, C' = (C - coff) * 255 / cmax with
gains rounded once in Q32 (luma) / Q28 (chroma, upsampled with weights summing to 16),
linear chroma upsampling (horizontally co-sited, vertically centred: chroma location 0),
edges clamped per tile; alpha = round(A * 255 / (2^bits - 1)); premultiplied colour clamped to
alpha.  Streams and ffmpeg's decodes are cached in <out>/heic_cache.
"""

import hashlib
import math
import random
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
import hevc_ref  # noqa: E402

C = it.C
Rec = it.Rec
OP_HEVCDEC, OP_HEICTIME, OP_HEVCENC = 160, 161, 162
OP_ENCMETA = 260                     # img_encode with an IMGMETA (png_ops.s)
STRAIGHT = C.IMGD_STRAIGHT

PIXFMT = {(0, 8): "gray", (0, 10): "gray10le", (0, 12): "gray12le",
          (1, 8): "yuv420p", (1, 10): "yuv420p10le", (1, 12): "yuv420p12le",
          (2, 8): "yuv422p", (2, 10): "yuv422p10le", (2, 12): "yuv422p12le",
          (3, 8): "yuv444p", (3, 10): "yuv444p10le", (3, 12): "yuv444p12le"}
SUB = {0: (1, 1), 1: (2, 2), 2: (2, 1), 3: (1, 1)}


# ================================================================== streams

class Stream:
    """An x265 stream of one picture with ffmpeg's decode: planes (u16 arrays), size."""

    def __init__(self, annexb, chroma, bits, w, h, planes):
        self.annexb, self.chroma, self.bits, self.w, self.h, self.planes = annexb, chroma, bits, w, h, planes
        self.nals = hevc_ref.split_annexb(annexb)


def gen_planes(seed, w, h, chroma, bits, kind="photo"):
    """Test content: smooth waves, edges and noise per plane (u16 arrays)."""
    rng = np.random.default_rng(seed)
    sw, sh = SUB[chroma]
    mx = (1 << bits) - 1
    out = []
    for c in range(1 if chroma == 0 else 3):
        pw, ph = (w, h) if c == 0 else (-(-w // sw), -(-h // sh))
        yy, xx = np.mgrid[0:ph, 0:pw].astype(np.float64)
        f1, f2 = 4 + rng.random() * 18, 3 + rng.random() * 25
        v = np.sin(xx / f1 + rng.random() * 6) * np.cos(yy / f2 + rng.random() * 6)
        v += 0.6 * ((xx // max(2, pw // 5) + yy // max(2, ph // 4)) % 2 - 0.5)
        if kind == "alpha":
            v = np.clip((xx / max(1, pw - 1)) * 2.4 - 0.7 + 0.3 * np.sin(yy / 5), 0, 1) * 2 - 1
        v = (v / 3 + 0.5) * mx + rng.normal(0, mx / 40, v.shape)
        out.append(np.clip(np.rint(v), 0, mx).astype(np.uint16))
    return out


def run_ff(args, inp=None):
    r = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"] + args, input=inp,
                       capture_output=True)
    if r.returncode:
        raise RuntimeError("ffmpeg failed: " + r.stderr.decode(errors="replace")[:400])
    return r.stdout


def make_stream(cache, w, h, chroma, bits, params="", seed=1, kind="photo", planes=None):
    """Encode planes (or generated content) with x265; cached together with ffmpeg's decode."""
    key = hashlib.sha1(repr((w, h, chroma, bits, params, seed, kind,
                             None if planes is None else [hashlib.sha1(p.tobytes()).hexdigest()
                                                         for p in planes])).encode()).hexdigest()[:16]
    cache.mkdir(parents=True, exist_ok=True)
    fs, fy = cache / f"{key}.hevc", cache / f"{key}.yuv"
    pf = PIXFMT[(chroma, bits)]
    if not (fs.exists() and fy.exists()):
        if planes is None:
            planes = gen_planes(seed, w, h, chroma, bits, kind)
        raw = b"".join((p.astype(np.uint8) if bits == 8 else p.astype("<u2")).tobytes() for p in planes)
        with tempfile.TemporaryDirectory() as td:
            ri = Path(td) / "in.raw"
            ri.write_bytes(raw)
            xp = "keyint=1:log-level=error" + (":" + params if params else "")
            run_ff(["-f", "rawvideo", "-pix_fmt", pf, "-s", f"{w}x{h}", "-i", str(ri), "-frames:v", "1",
                    "-c:v", "libx265", "-x265-params", xp, "-f", "hevc", str(fs)])
        run_ff(["-i", str(fs), "-f", "rawvideo", "-pix_fmt", pf, str(fy)])
    yuv = fy.read_bytes()
    dt = np.uint8 if bits == 8 else np.dtype("<u2")
    a = np.frombuffer(yuv, dt).astype(np.uint16)
    sw, sh = SUB[chroma]
    out, p = [], 0
    for c in range(1 if chroma == 0 else 3):
        pw, ph = (w, h) if c == 0 else (-(-w // sw), -(-h // sh))
        out.append(a[p:p + pw * ph].reshape(ph, pw))
        p += pw * ph
    return Stream(fs.read_bytes(), chroma, bits, w, h, out)


# ================================================================== YUV -> RGB model (yuv.s)

MTAB = {1: (0.2126, 0.0722), 4: (0.30, 0.11), 5: (0.299, 0.114), 6: (0.299, 0.114),
        7: (0.212, 0.087), 9: (0.2627, 0.0593), 10: (0.2627, 0.0593)}
PTAB = {1: (0.2126390058715103, 0.07219231536073373), 4: (0.2989666181247899, 0.1146121717422266),
        5: (0.22200430999823087, 0.07134092407648639), 6: (0.2123763607050675, 0.08656378236920956),
        7: (0.2123763607050675, 0.08656378236920956), 8: (0.2535853634337347, 0.06807886086929262),
        9: (0.262700212011267, 0.05930171646986194), 11: (0.20949167791273057, 0.06891306792622581),
        12: (0.22897456406974873, 0.079286914093745), 22: (0.23175054567210923, 0.09599868152322844)}


def krkb(m, prim):
    if m in (12, 13):
        v = PTAB.get(prim)
        return v if v else (0.299, 0.114)
    return MTAB.get(m, (0.299, 0.114))


def coef(bits, bitsc, cicp):
    m, full, prim = (cicp >> 16) & 255, (cicp >> 24) & 1, cicp & 255
    if full:
        ymax, yoff = (1 << bits) - 1, 0
    else:
        ymax, yoff = 219 << (bits - 8), 16 << (bits - 8)
    ky = math.floor(255.0 / ymax * 4294967296.0 + 0.5)
    if full:
        cmax, coff = (1 << bitsc) - 1, (0 if m == 0 else 1 << (bitsc - 1))
    else:
        cmax = (219 if m == 0 else 224) << (bitsc - 8)
        coff = (16 if m == 0 else 128) << (bitsc - 8)
    cmul = 255.0 / cmax

    def r28(g):
        return math.floor(g * 268435456.0 + 0.5)
    if m == 0:
        one = r28(cmul)
        k = ((0, 0, one), (ky, 0, 0), (0, one, 0))
    elif m == 8:
        one = r28(cmul)
        k = ((ky, -one, one), (ky, one, 0), (ky, -one, -one))
    else:
        kr, kb = krkb(m, prim)
        kr2 = 2.0 - (kr + kr)
        kb2 = 2.0 - (kb + kb)
        kg = (1.0 - kr) - kb
        gcb = ((kb + kb) * (1.0 - kb)) / kg
        gcr = ((kr + kr) * (1.0 - kr)) / kg
        k = ((ky, 0, r28(cmul * kr2)), (ky, -r28(cmul * gcb), -r28(cmul * gcr)), (ky, r28(cmul * kb2), 0))
    return k, yoff, coff * 16, ky


def upsample(c, w, h, sw, sh, loc=0, nearest=False):
    """Chroma plane (ch, cw) -> (h, w) int64 times 16 (yuv.s weights)."""
    c = c.astype(np.int64)
    ch, cw = c.shape
    y = np.arange(h)
    if sh == 1:
        v = 4 * c[y]
    elif nearest:
        v = 4 * c[y >> 1]
    elif loc in (2, 3):
        k = y >> 1
        k1 = np.minimum(k + 1, ch - 1)
        v = np.where((y & 1)[:, None] == 0, 4 * c[k], 2 * c[k] + 2 * c[k1])
    else:
        k = y >> 1
        kn = np.where(y & 1, np.minimum(k + 1, ch - 1), np.maximum(k - 1, 0))
        v = 3 * c[k] + c[kn]
    x = np.arange(w)
    if sw == 1:
        return 4 * v[:, x]
    if nearest:
        return 4 * v[:, x >> 1]
    k = x >> 1
    if loc in (0, 2, 4):
        k1 = np.minimum(k + 1, cw - 1)
        return np.where((x & 1)[None, :] == 0, 4 * v[:, k], 2 * v[:, k] + 2 * v[:, k1])
    kn = np.where(x & 1, np.minimum(k + 1, cw - 1), np.maximum(k - 1, 0))
    return 3 * v[:, k] + v[:, kn]


def yuv_to_bgra(planes, chroma, bits, bitsc, cicp, loc=0, nearest=False):
    yp = planes[0].astype(np.int64)
    h, w = yp.shape
    k, yoff, coff16, ky = coef(bits, bitsc, cicp)
    out = np.empty((h, w, 4), np.uint8)
    out[..., 3] = 255
    yv = yp - yoff
    if chroma == 0:
        g = np.clip((yv * ky + (1 << 31)) >> 32, 0, 255)
        out[..., 0] = out[..., 1] = out[..., 2] = g
        return out
    sw, sh = SUB[chroma]
    u = upsample(planes[1], w, h, sw, sh, loc, nearest) - coff16
    v = upsample(planes[2], w, h, sw, sh, loc, nearest) - coff16
    for ch, (a, b, c) in zip((2, 1, 0), k):
        out[..., ch] = np.clip((yv * a + u * b + v * c + (1 << 31)) >> 32, 0, 255)
    return out


def alpha8(plane, bits):
    ka = math.floor(255.0 / ((1 << bits) - 1) * 4294967296.0 + 0.5)
    return np.minimum((plane.astype(np.int64) * ka + (1 << 31)) >> 32, 255).astype(np.uint8)


def stream_cicp(st):
    """CICP the decoder takes from the stream's VUI (colour description or 2/2/2, full range flag)."""
    sps = next(n for n in st.nals if (n[0] >> 1) & 63 == 33)
    s = hevc_ref.parse_sps(hevc_ref.unescape(sps)[2:])
    col = getattr(s, "vui_colour", None) or (2, 2, 2)
    return col[0] | col[1] << 8 | col[2] << 16 | (getattr(s, "vui_full_range", 0) & 1) << 24


# ================================================================== HEIF writer

def box(t, payload=b"", large=False):
    return it.bmff_box(t, payload, large)


def fbox(t, v, f, payload=b""):
    return it.bmff_fullbox(t, v, f, payload)


def unescape(nal):
    out, z = bytearray(), 0
    for b in nal:
        if z >= 2 and b == 3:
            z = 0
            continue
        out.append(b)
        z = z + 1 if b == 0 else 0
    return bytes(out)


def hvcc(stream, lsz=4, extra_arrays=()):
    ps = [n for n in stream.nals if (n[0] >> 1) & 63 in (32, 33, 34)]
    sps = unescape(next(n for n in ps if (n[0] >> 1) & 63 == 33))
    ptl = sps[3:15]
    b = bytes([1]) + ptl + struct.pack(">HBBBBHB", 0xF000, 0xFC, 0xFC | stream.chroma,
                                       0xF8 | (stream.bits - 8), 0xF8 | (stream.bits - 8), 0,
                                       0x0C | (lsz - 1))
    groups = {}
    for n in ps:
        groups.setdefault((n[0] >> 1) & 63, []).append(n)
    arrays = sorted(groups.items()) + list(extra_arrays)
    b += bytes([len(arrays)])
    for t, ns in arrays:
        b += bytes([0x80 | t]) + struct.pack(">H", len(ns))
        for n in ns:
            b += struct.pack(">H", len(n)) + n
    return box(b"hvcC", b)


def coded_data(stream, lsz=4):
    """Length-prefixed NAL units of the picture (SEI dropped for 1-byte lengths)."""
    skip = (32, 33, 34, 39, 40) if lsz == 1 else (32, 33, 34)
    return b"".join(len(n).to_bytes(lsz, "big") + n for n in stream.nals if (n[0] >> 1) & 63 not in skip)


def ispe(w, h):
    return fbox(b"ispe", 0, 0, struct.pack(">II", w, h))


def nclx(p, t, m, full):
    return box(b"colr", b"nclx" + struct.pack(">HHHB", p, t, m, full << 7))


def clap(wn, wd, hn, hd, xn, xd, yn, yd):
    return box(b"clap", struct.pack(">IIIIiIiI", wn, wd, hn, hd, xn, xd, yn, yd))


URN_HEVC = b"urn:mpeg:hevc:2015:auxid:1"
URN_MPEG = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha"


class Heif:
    """Minimal HEIF writer: items (id, type, data), properties, associations, references."""

    def __init__(self):
        self.items = []          # dict(id, type, data, ctype, hidden, cm, split)
        self.props = []
        self.assoc = {}
        self.refs = []

    def item(self, typ, data, ctype=None, hidden=False, cm=0, split=1, iid=None):
        iid = iid or (len(self.items) + 1)
        self.items.append(dict(id=iid, type=typ, data=bytes(data), ctype=ctype, hidden=hidden, cm=cm,
                               split=split))
        return iid

    def prop(self, b, iid=None, ess=False):
        if b in self.props:
            k = self.props.index(b) + 1
        else:
            self.props.append(b)
            k = len(self.props)
        if iid is not None:
            self.assoc.setdefault(iid, []).append((k, ess))
        return k

    def ref(self, typ, frm, tos):
        self.refs.append((typ, frm, list(tos)))

    def build(self, primary, o=None):
        """o: dict of variants: iloc_v (0/1/2), osz / lsz / bsz (4/8), base (int), infe_v (2/3),
        pitm_v, ipma_v, ipma_f, large (box sizes), mdat_first, hdlr, ftyp_brands, no_pitm,
        idat (put cm=1 items into idat)."""
        o = o or {}
        big_ids = o.get("infe_v", 2) == 3 or o.get("ipma_v", 0) == 1 or o.get("iloc_v", 1) == 2
        idw = 4 if big_ids else 2

        def u_id(v, n=None):
            return v.to_bytes(n or idw, "big")
        # infe / iinf
        infes = b""
        for itm in self.items:
            v = o.get("infe_v", 2)
            body = (struct.pack(">I", itm["id"]) if v == 3 else struct.pack(">H", itm["id"])) + b"\0\0"
            body += itm["type"] + b"\0"
            if itm["type"] == b"mime":
                body += itm["ctype"] + b"\0"
            infes += fbox(b"infe", v, 1 if itm["hidden"] else 0, body)
        iinf = fbox(b"iinf", 0, 0, struct.pack(">H", len(self.items)) + infes) if len(self.items) < 65536 else None
        # iref
        iref = b""
        if self.refs:
            v = 1 if big_ids else 0
            body = b""
            for typ, frm, tos in self.refs:
                body += box(typ, u_id(frm) + struct.pack(">H", len(tos)) + b"".join(u_id(t) for t in tos))
            iref = fbox(b"iref", v, 0, body)
        # iprp
        ipco = box(b"ipco", b"".join(self.props))
        ipv, ipf = o.get("ipma_v", 0), o.get("ipma_f", 0)
        body = struct.pack(">I", len(self.assoc))
        for iid, lst in self.assoc.items():
            body += (struct.pack(">I", iid) if ipv else struct.pack(">H", iid)) + bytes([len(lst)])
            for k, ess in lst:
                body += struct.pack(">H", (ess << 15) | k) if ipf & 1 else bytes([(ess << 7) | k])
        iprp = box(b"iprp", ipco + fbox(b"ipma", ipv, ipf, body))
        hdlr = fbox(b"hdlr", 0, 0, b"\0\0\0\0" + o.get("hdlr", b"pict") + b"\0" * 12 + b"\0")
        pv = o.get("pitm_v", 1 if primary > 65535 else 0)
        pitm = b"" if o.get("no_pitm") else fbox(b"pitm", pv, 0, struct.pack(">I" if pv else ">H", primary))
        # data layout: idat items (cm 1) and mdat items (cm 0)
        idat_body = b""
        mdat_body = b""
        places = {}
        for itm in self.items:
            d = itm["data"]
            if itm["cm"] == 1:
                places[itm["id"]] = ("idat", len(idat_body))
                idat_body += d
            else:
                places[itm["id"]] = ("mdat", len(mdat_body))
                mdat_body += d
        ilv = o.get("iloc_v", 1)
        osz, lsz, bsz = o.get("osz", 4), o.get("lsz", 4), o.get("bsz", 0)
        base_extra = o.get("base", 0)

        def iloc_box(mdat_off):
            body = bytes([(osz << 4) | lsz, (bsz << 4) | (4 if ilv and o.get("isz") else 0)])
            body += struct.pack(">I" if ilv == 2 else ">H", len(self.items))
            for itm in self.items:
                where, off = places[itm["id"]]
                body += u_id(itm["id"], 4 if ilv == 2 else 2)
                if ilv:
                    body += struct.pack(">H", itm["cm"])
                body += b"\0\0"
                start = off + (mdat_off if where == "mdat" else 0)
                base = 0
                if bsz:
                    base = min(start, base_extra) if base_extra else 0
                    body += base.to_bytes(bsz, "big")
                n = len(itm["data"])
                parts = itm["split"]
                cuts = [n * i // parts for i in range(parts + 1)]
                body += struct.pack(">H", parts)
                for i in range(parts):
                    if ilv and o.get("isz"):
                        body += struct.pack(">I", 0)
                    eo, el = start + cuts[i] - base, cuts[i + 1] - cuts[i]
                    if o.get("len0") and i == parts - 1 and where == "mdat" and itm is self.items[-1]:
                        el = 0
                    body += eo.to_bytes(osz, "big") + el.to_bytes(lsz, "big")
            return fbox(b"iloc", ilv, 0, body)
        idat = box(b"idat", idat_body) if idat_body else b""
        brands = o.get("ftyp_brands", (b"heic", [b"mif1", b"heic"]))
        ft = it.ftyp(brands[0], 0, brands[1])
        large = o.get("large", False)

        def meta_box(mdat_off):
            children = [hdlr, pitm, iinf, iloc_box(mdat_off), iprp, iref, idat]
            if o.get("meta_order") == "rev":
                children = children[::-1]
            return box(b"meta", b"\0\0\0\0" + b"".join(children), large)
        if o.get("mdat_first"):
            mdat_hdr = 16 if large else 8
            mdat_off = len(ft) + mdat_hdr
            mdat = box(b"mdat", mdat_body, large)
            return ft + mdat + meta_box(mdat_off)
        m0 = meta_box(0)
        mdat_off = len(ft) + len(m0) + (16 if large else 8)
        mb = meta_box(mdat_off)
        assert len(mb) == len(m0)
        return ft + mb + box(b"mdat", mdat_body, large)


# ================================================================== cases

class Case:
    def __init__(self, name, data, exp=None, flags=STRAIGHT, fmt=None, meta=None, err=None, check=None):
        self.name, self.data, self.exp, self.flags, self.fmt, self.meta, self.err = name, data, exp, flags, fmt, meta, err
        self.check = check

    def rec(self):
        if self.check:
            chk = self.check
        elif self.err is not None:
            chk = it.expect_err(*([self.err] if isinstance(self.err, int) else self.err))
        else:
            chk = it.expect_decex(self.exp, self.fmt, 0, self.meta)
        return Rec(it.OP_DECEX, 0, it.decex_input(self.data, flags=self.flags), chk, self.name)


def single(st, h=None, cicp_box=None, extra_props=(), lsz=4, o=None, split=1):
    h = h or Heif()
    iid = h.item(b"hvc1", coded_data(st, lsz), split=split)
    h.prop(hvcc(st, lsz), iid, True)
    h.prop(ispe(st.w, st.h), iid)
    if cicp_box:
        h.prop(cicp_box, iid)
    for p in extra_props:
        h.prop(p, iid, True)
    return h, iid


def wic_conv(planes, chroma, bits, bitsc, cicp):
    """What Windows' HEIF decoder does (measured): BT.709 whatever nclx / VUI say, their range."""
    return yuv_to_bgra(planes, chroma, bits, bitsc, (cicp & 0xFF000000) | 0x010101)


CONV = [yuv_to_bgra]                 # model used by build_cases (wic suite: wic_conv)


def model_single(st, cicp):
    return CONV[0](st.planes, st.chroma, st.bits, st.bits, cicp)


def apply_props(img, props):
    """Expected display image: clap / irot / imir in order -> (cropped stored image, orientation)."""
    o = 1
    comp = [[1, 2, 3, 4, 5, 6, 7, 8], [2, 1, 4, 3, 8, 7, 6, 5], [3, 4, 1, 2, 7, 8, 5, 6], [4, 3, 2, 1, 6, 5, 8, 7],
            [5, 6, 7, 8, 1, 2, 3, 4], [6, 5, 8, 7, 4, 3, 2, 1], [7, 8, 5, 6, 3, 4, 1, 2], [8, 7, 6, 5, 2, 1, 4, 3]]
    x0, y0 = 0, 0
    hh, ww = img.shape[:2]
    for p in props:
        if p[0] == "irot":
            o = comp[o - 1][[1, 8, 3, 6][p[1]] - 1]
        elif p[0] == "imir":
            o = comp[o - 1][(2 if p[1] else 4) - 1]      # axis 0: top / bottom (23008-12:2022)
        elif p[0] == "clap":
            wn, wd, hn, hd, xn, xd, yn, yd = p[1]
            disp = it.orient(np.zeros((hh, ww), np.uint8), o)
            dh, dw = disp.shape
            cw = (2 * wn + wd) // (2 * wd)
            chh = (2 * hn + hd) // (2 * hd)
            left = (2 * xn * wd + (dw * wd - wn) * xd) // (2 * xd * wd)
            top = (2 * yn * hd + (dh * hd - hn) * yd) // (2 * yd * hd)
            right, bottom = left + cw - 1, top + chh - 1
            left, top = max(left, 0), max(top, 0)
            right, bottom = min(right, dw - 1), min(bottom, dh - 1)
            # map the display rectangle back through the orientation (index grid)
            idx = it.orient(np.arange(hh * ww).reshape(hh, ww), o)[top:bottom + 1, left:right + 1]
            ys, xs = idx // ww, idx % ww
            nx0, ny0 = xs.min(), ys.min()
            x0, y0 = x0 + nx0, y0 + ny0
            ww, hh = xs.max() - nx0 + 1, ys.max() - ny0 + 1
    return (x0, y0, ww, hh), o


def prop_boxes(props):
    out = []
    for p in props:
        if p[0] == "irot":
            out.append(box(b"irot", bytes([p[1]])))
        elif p[0] == "imir":
            out.append(box(b"imir", bytes([p[1]])))
        elif p[0] == "clap":
            out.append(clap(*p[1]))
    return out


def build_cases(cache, seed):
    rng = random.Random(seed)
    cases = []
    S = {}

    def st(name, *a, **k):
        if name not in S:
            S[name] = make_stream(cache, *a, **k)
        return S[name]

    base = st("base", 96, 64, 1, 8, seed=3)
    # --- colour: nclx matrices / ranges, VUI, nothing
    for (p, t, m, full) in [(1, 13, 6, 1), (1, 1, 1, 0), (1, 13, 1, 1), (9, 16, 9, 0), (2, 2, 2, 0),
                            (5, 6, 5, 1), (4, 4, 4, 0), (7, 7, 7, 1), (12, 13, 12, 1), (1, 13, 12, 0),
                            (9, 1, 10, 1), (2, 2, 8, 1), (2, 2, 8, 0), (1, 1, 3, 1), (1, 1, 99, 0)]:
        cicp = p | t << 8 | m << 16 | full << 24
        h, _ = single(base, cicp_box=nclx(p, t, m, full))
        cases.append(Case(f"nclx {p}/{t}/{m}/{full}", h.build(1), model_single(base, cicp),
                          meta={"w": 96, "h": 64, "cicp": cicp}))
    g444 = st("g444", 64, 48, 3, 8, seed=4)
    for full in (0, 1):
        h, _ = single(g444, cicp_box=nclx(1, 13, 0, full))
        cases.append(Case(f"identity full={full}", h.build(1), model_single(g444, 1 | 13 << 8 | full << 24)))
    for params in ["colorprim=bt709:transfer=bt709:colormatrix=bt709:range=full",
                   "colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc", "range=full", ""]:
        s = st("vui" + params, 80, 48, 1, 8, params=params, seed=5)
        h, _ = single(s)
        cases.append(Case(f"vui '{params}'", h.build(1), model_single(s, stream_cicp(s))))
    # --- formats
    for name, (w, hh, ch, bd, prm) in {
            "420-10": (72, 40, 1, 10, "profile=main10"), "420-12": (40, 24, 1, 12, "profile=main12"),
            "422-10": (64, 32, 2, 10, "profile=main422-10"), "444-8": (48, 40, 3, 8, "profile=main444-8"),
            "444-12": (32, 32, 3, 12, "profile=main444-12"), "400-8": (56, 40, 0, 8, ""),
            "400-10": (40, 40, 0, 10, ""), "odd": (202, 118, 1, 8, ""), "422-8": (48, 32, 2, 8, ""),
            "lossless": (48, 32, 1, 8, "lossless=1"), "tiny": (16, 16, 1, 8, ""),
            "wpp": (160, 128, 1, 8, "wpp=1:ctu=16"), "slices": (128, 96, 1, 8, "slices=3:ctu=16")}.items():
        s = st(name, w, hh, ch, bd, params=prm, seed=6)
        h, _ = single(s)
        cases.append(Case(f"fmt {name}", h.build(1), model_single(s, stream_cicp(s)),
                          meta={"bits": bd, "w": w, "h": hh, "channels": 1 if ch == 0 else 3,
                                "model": C.IMM_GRAY if ch == 0 else C.IMM_YCC}))
    # --- transformations
    plist = [[("irot", 1)], [("irot", 2)], [("irot", 3)], [("imir", 0)], [("imir", 1)],
             [("irot", 1), ("imir", 0)], [("imir", 1), ("irot", 3)],
             [("clap", (64, 1, 40, 1, 0, 1, 0, 1))], [("clap", (95, 1, 63, 1, -1, 2, -1, 2))],
             [("clap", (50, 1, 30, 1, -11, 1, 7, 1))], [("clap", (99, 2, 61, 2, 3, 4, -5, 3))],
             [("clap", (50, 1, 30, 1, -11, 1, 7, 1)), ("irot", 1)],
             [("irot", 1), ("clap", (30, 1, 50, 1, 7, 1, -11, 1))],
             [("imir", 0), ("clap", (40, 1, 20, 1, 9, 1, 3, 1)), ("irot", 3)],
             [("irot", 2), ("imir", 1), ("clap", (60, 1, 40, 1, -5, 1, 2, 1)), ("irot", 1)],
             [("clap", (500, 1, 500, 1, 0, 1, 0, 1))]]
    full_img = model_single(base, stream_cicp(base))
    for props in plist:
        h, _ = single(base, extra_props=prop_boxes(props))
        (x0, y0, cw, chh), o = apply_props(full_img, props)
        exp = it.orient(full_img[y0:y0 + chh, x0:x0 + cw], o)
        cases.append(Case(f"props {props}", h.build(1), exp, meta={"w": cw, "h": chh, "orient": o}))
        if props == plist[13]:
            cases.append(Case(f"no-orient {props}", h.build(1), full_img[y0:y0 + chh, x0:x0 + cw],
                              flags=STRAIGHT | C.IMGD_NO_ORIENT))
    # --- grids: tiles cut from one source, the last row / column clipped
    for (tw, th, cols, rows, ow, oh, ch, bd) in [(64, 48, 3, 2, 170, 90, 1, 8), (32, 32, 2, 2, 64, 64, 1, 10),
                                                 (48, 32, 2, 3, 80, 90, 3, 8), (40, 24, 1, 1, 33, 20, 0, 8)]:
        src = gen_planes(rng.randrange(1 << 30), tw * cols, th * rows, ch, bd)
        sw, sh = SUB[ch]
        h = Heif()
        gid = h.item(b"grid", b"")
        tiles = []
        exp = np.zeros((oh, ow, 4), np.uint8)
        for r in range(rows):
            for c in range(cols):
                tp = [src[0][r * th:(r + 1) * th, c * tw:(c + 1) * tw]]
                if ch:
                    tp += [p[r * th // sh:(r + 1) * th // sh, c * tw // sw:(c + 1) * tw // sw] for p in src[1:]]
                ts = make_stream(cache, tw, th, ch, bd, planes=tp)
                tid = h.item(b"hvc1", coded_data(ts), hidden=True)
                h.prop(hvcc(ts), tid, True)
                h.prop(ispe(tw, th), tid)
                tiles.append(tid)
                rgb = model_single(ts, stream_cicp(ts))
                y1, x1 = min(oh, (r + 1) * th), min(ow, (c + 1) * tw)
                if r * th < oh and c * tw < ow:
                    exp[r * th:y1, c * tw:x1] = rgb[:y1 - r * th, :x1 - c * tw]
        fl = 1 if ow > 65535 else 0
        gd = bytes([0, fl, rows - 1, cols - 1]) + struct.pack(">HH", ow, oh)
        h.items[0]["data"] = gd
        h.ref(b"dimg", gid, tiles)
        h.prop(ispe(ow, oh), gid)
        cases.append(Case(f"grid {cols}x{rows} {tw}x{th} -> {ow}x{oh} ch{ch} bd{bd}", h.build(gid), exp,
                          meta={"w": ow, "h": oh, "bits": bd}))
        if bd == 8 and ch == 1:
            h.items[0]["cm"] = 1          # grid descriptor in idat
            h.items[0]["data"] = bytes([0, 1, rows - 1, cols - 1]) + struct.pack(">II", ow, oh)
            cases.append(Case("grid idat 32-bit", h.build(gid), exp))
    # --- alpha
    a8 = st("alpha8", 96, 64, 0, 8, seed=7, kind="alpha")
    a10 = st("alpha10", 96, 64, 0, 10, seed=8, kind="alpha")
    for urn, ast, prem in [(URN_HEVC, a8, False), (URN_MPEG, a8, False), (URN_HEVC, a10, False),
                           (URN_HEVC, a8, True)]:
        h, cid = single(base)
        aid = h.item(b"hvc1", coded_data(ast))
        h.prop(hvcc(ast), aid, True)
        h.prop(ispe(96, 64), aid)
        h.prop(fbox(b"auxC", 0, 0, urn + b"\0"), aid, True)
        h.ref(b"auxl", aid, [cid])
        exp = model_single(base, stream_cicp(base)).copy()
        exp[..., 3] = alpha8(ast.planes[0], ast.bits)
        flags, fmt, mf = STRAIGHT, C.FMT_ARGB, C.IMF_ALPHA
        if prem:
            h.ref(b"prem", cid, [aid])
            exp[..., :3] = np.minimum(exp[..., :3], exp[..., 3:4])
            flags, fmt, mf = 0, C.FMT_PARGB, C.IMF_ALPHA | C.IMF_SRC_PREMUL
        cases.append(Case(f"alpha {urn[-12:]} bd{ast.bits} prem={prem}", h.build(cid), exp, flags=flags,
                          fmt=fmt, meta={"channels": 4}))
    h, cid = single(base)
    aid = h.item(b"hvc1", coded_data(a8))
    h.prop(hvcc(a8), aid, True)
    h.prop(fbox(b"auxC", 0, 0, b"urn:mpeg:hevc:2015:auxid:2\0"), aid, True)     # depth: not alpha
    h.ref(b"auxl", aid, [cid])
    cases.append(Case("aux depth ignored", h.build(cid), model_single(base, stream_cicp(base)), meta={"channels": 3}))
    # --- container variants
    variants = [dict(iloc_v=0), dict(iloc_v=2), dict(osz=8, lsz=8), dict(bsz=4, base=10), dict(bsz=8, base=3),
                dict(infe_v=3), dict(pitm_v=1), dict(ipma_v=1), dict(ipma_f=1), dict(large=True),
                dict(mdat_first=True), dict(meta_order="rev"), dict(iloc_v=1, isz=1), dict(len0=1),
                dict(ftyp_brands=(b"mif1", [b"mif1", b"heic"]))]
    for v in variants:
        h, _ = single(base)
        cases.append(Case(f"container {v}", h.build(1, v), model_single(base, stream_cicp(base))))
    for lsz in (1, 2):
        sm = st("small", 32, 24, 1, 8, params="ctu=16:qp=44", seed=9) if lsz == 1 else base
        h, _ = single(sm, lsz=lsz)
        cases.append(Case(f"nal length {lsz}", h.build(1), model_single(sm, stream_cicp(sm))))
    h, _ = single(base, split=5)
    cases.append(Case("5 extents", h.build(1), model_single(base, stream_cicp(base))))
    h, _ = single(base, split=3)
    h.items[0]["cm"] = 1
    cases.append(Case("3 extents in idat", h.build(1), model_single(base, stream_cicp(base))))
    h, _ = single(base)
    h.items[0]["cm"] = 1
    cases.append(Case("coded data in idat", h.build(1), model_single(base, stream_cicp(base))))
    return cases, S


# ================================================================== suites

def suite_hevc(runners, cache, suites):
    """Raw streams through op 160 against ffmpeg's planes."""
    specs = [(64, 64, 1, 8, ""), (202, 118, 1, 8, ""), (128, 96, 1, 8, "ctu=16:slices=4"),
             (160, 128, 1, 8, "wpp=1:ctu=32"), (96, 64, 1, 8, "tskip=1:rdoq-level=2"),
             (64, 48, 1, 8, "lossless=1"), (96, 64, 1, 8, "cbqpoffs=4:crqpoffs=-3:aq-mode=3"),
             (72, 40, 1, 10, "profile=main10"), (40, 24, 1, 12, "profile=main12"),
             (64, 32, 2, 10, "profile=main422-10"), (48, 40, 3, 8, "profile=main444-8"),
             (32, 32, 3, 12, "profile=main444-12"), (56, 40, 0, 8, ""), (40, 40, 0, 10, ""),
             (96, 64, 1, 8, "no-sao=1:no-deblock=1"), (96, 64, 1, 8, "qp=51"), (96, 64, 1, 8, "qp=1"),
             (256, 192, 1, 8, "ctu=64:max-tu-size=32:tu-intra-depth=4"), (66, 34, 1, 8, "ctu=16:min-cu-size=8"),
             (120, 80, 1, 8, "signhide=0:strong-intra-smoothing=0:constrained-intra=1")]
    recs = []
    for (w, h, ch, bd, prm) in specs:
        s = make_stream(cache, w, h, ch, bd, params=prm, seed=w * 7 + h)

        def chk(res, out, s=s):
            if res != 0:
                return f"error {it.err_name(res)}"
            out = bytes(out)
            ww, hh, b1, b2, cf = struct.unpack_from("<5I", out, 0)
            got = np.frombuffer(out, np.uint16, offset=20)
            ref = np.concatenate([p.ravel() for p in s.planes])
            if (ww, hh, b1, cf) != (s.w, s.h, s.bits, s.chroma) or len(got) != len(ref):
                return f"geometry {ww}x{hh} bd{b1} cf{cf} n{len(got)} vs {s.w}x{s.h} bd{s.bits} cf{s.chroma}"
            d = np.nonzero(got != ref)[0]
            return None if len(d) == 0 else f"{len(d)} samples differ (first at {d[0]})"
        recs.append(Rec(OP_HEVCDEC, 0, s.annexb, chk, f"{w}x{h} cf{ch} bd{bd} {prm}"))
    for t, r in runners.items():
        s = it.Suite("hevc", t)
        it.run_suite(r, s, recs)
        suites.append(s)


def suite_cases(runners, cases, suites, name):
    recs = [c.rec() for c in cases]
    for t, r in runners.items():
        s = it.Suite(name, t)
        it.run_suite(r, s, recs)
        suites.append(s)


def meta_cases(S):
    base = S["base"]
    out = []
    icc = bytes(range(256)) * 3
    tiff = it.tiff_bytes([{"entries": [(0x0112, 3, [6]), (0x010F, 2, b"heic_check\0")], "next": None}])
    xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/">heic</x:xmpmeta>'
    for exif_off in (0, 6):
        h, cid = single(base, cicp_box=box(b"colr", b"prof" + icc))
        h.prop(nclx(1, 13, 1, 1), cid)
        eid = h.item(b"Exif", struct.pack(">I", exif_off) + b"Exif\0\0"[:exif_off] + tiff)
        h.ref(b"cdsc", eid, [cid])
        xid = h.item(b"mime", xmp, ctype=b"application/rdf+xml")
        h.ref(b"cdsc", xid, [cid])
        data = h.build(cid)

        def chk(res, out, exif_off=exif_off):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            if d["icc"] != icc:
                return f"icc {len(d['icc'])} bytes"
            if d["exif"] != tiff:
                return f"exif {len(d['exif'])} bytes"
            if d["xmp"] != xmp:
                return f"xmp {d['xmp'][:40]!r}"
            m = d["meta"]
            if m["orient"] != 1:
                return f"orient {m['orient']} (EXIF orientation must be ignored)"
            if m["cicp"] != (1 | 13 << 8 | 1 << 16 | 1 << 24):
                return f"cicp {m['cicp']:#x}"
            return None
        out.append(Case(f"blobs exif_off={exif_off}", data, flags=STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP,
                        check=chk))

        def chk_probe(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            m = d["meta"]
            if d["px"] is not None:
                return "pixels returned for IMGD_PROBE"
            exp = dict(w=96, h=64, bits=8, model=C.IMM_YCC, channels=3, frames=1)
            bad = {k: m[k] for k in exp if m[k] != exp[k]}
            return f"meta {bad}" if bad else None
        out.append(Case(f"probe exif_off={exif_off}", data, flags=STRAIGHT | C.IMGD_PROBE, check=chk_probe))
    # without cdsc: still found
    h, cid = single(base)
    h.item(b"Exif", struct.pack(">I", 0) + tiff)
    out.append(Case("exif without cdsc", h.build(cid), flags=STRAIGHT | C.IMGD_EXIF,
                    check=lambda r, o: None if r == 0 and it.parse_decex(o)["exif"] == tiff else "exif missing"))
    return out


def error_cases(S):
    base = S["base"]
    out = []
    h, _ = single(base)
    good = h.build(1)
    out.append(Case("no pitm", single(base)[0].build(1, dict(no_pitm=True)), err=C.ERR_HDR))
    out.append(Case("hdlr vide", single(base)[0].build(1, dict(hdlr=b"vide")), err=C.ERR_UNSUPPORTED))
    out.append(Case("pitm unknown id", single(base)[0].build(7), err=C.ERR_HDR))
    h, _ = single(base)
    h.items[0]["type"] = b"av01"
    out.append(Case("av01 item without av1C", h.build(1), err=C.ERR_HDR))
    h, _ = single(base)
    h.items[0]["type"] = b"iovl"
    out.append(Case("iovl item", h.build(1), err=C.ERR_UNSUPPORTED))
    out.append(Case("truncated mdat", good[:-40], err=(C.ERR_TRUNC,)))
    h = Heif()
    iid = h.item(b"hvc1", coded_data(base))
    h.prop(ispe(96, 64), iid)
    out.append(Case("no hvcC", h.build(1), err=C.ERR_HDR))
    h = Heif()
    gid = h.item(b"grid", bytes([0, 0, 1, 1]) + struct.pack(">HH", 192, 128))
    tid = h.item(b"hvc1", coded_data(base))
    h.prop(hvcc(base), tid)
    h.ref(b"dimg", gid, [tid, tid, tid])
    h.prop(ispe(192, 128), gid)
    out.append(Case("grid with 3 of 4 tiles", h.build(gid), err=C.ERR_HDR))
    h, _ = single(base)
    h.props[0] = h.props[0][:20]          # hvcC cut below 23 bytes
    out.append(Case("short hvcC", h.build(1), err=C.ERR_HDR))
    out.append(Case("no meta", it.ftyp(b"heic", 0, [b"mif1"]) + box(b"mdat", b"\0" * 32), err=C.ERR_HDR))
    return out


def mutants(cases, n, seed):
    rng = random.Random(seed)
    src = [c.data for c in cases if c.err is None and len(c.data) < 200000]
    out = []
    for i in range(n):
        d = bytearray(rng.choice(src))
        k = rng.random()
        if k < 0.25:
            d = d[:rng.randrange(len(d))]
        elif k < 0.6:
            for _ in range(rng.randrange(1, 6)):
                p = rng.randrange(min(len(d), 1200))     # headers / meta mostly
                d[p] = rng.randrange(256)
        elif k < 0.8:
            for _ in range(rng.randrange(1, 4)):
                p = rng.randrange(len(d))
                d[p] ^= 1 << rng.randrange(8)
        else:
            p = rng.randrange(min(len(d), 800))
            d[p:p + 4] = struct.pack(">I", rng.choice([0, 1, 0xFFFFFFFF, 0x7FFFFFFF, rng.randrange(1 << 32)]))
        out.append(Rec(it.OP_DECEX, 0, it.decex_input(bytes(d), flags=STRAIGHT | 14), it.any_result, f"mut{i}"))
    return out


def suite_fuzz(runners, cases, suites, n, seed):
    recs = mutants(cases, n, seed)
    for t, r in runners.items():
        s = it.Suite("fuzz", t)
        it.run_suite(r, s, recs)
        suites.append(s)


def suite_wic(cases, cache, seed, out, suites):
    """Windows' decoder on the heif cases against the same pipeline with wic_conv: catches
    gross errors (tiles, transformations); WIC filters chroma differently (25..34 dB on the
    noisy test content), wrong geometry gives < 15 dB, so PSNR >= 22 dB is required."""
    s = it.Suite("wic", "win")
    CONV[0] = wic_conv
    try:
        wcases = {c.name: c for c in build_cases(cache, seed)[0]}
    finally:
        CONV[0] = yuv_to_bgra
    work = out / "heic_wic"
    work.mkdir(parents=True, exist_ok=True)
    wic = it.Wic(work)
    # WIC picks the matrix from the primaries (1: BT.709, else BT.601) and colour-manages
    # P3 / BT.2020 / PQ; it handles fractional clean apertures differently and ignores alpha
    # and applies transformations in the MIAF order (clap, irot, imir) instead of the ipma order
    skip = ("nclx 2/", "nclx 4/", "nclx 5/", "nclx 7/", "nclx 9/", "nclx 12/", "/8/", "/0/", "identity",
            "smpte2084", "(95, 1, 63", "(99, 2,", "no-orient", "prem=True", "[('imir', 1), ('irot'",
            "[('imir', 0), ('clap'", "[('irot', 2), ('imir'", "[('irot', 1), ('clap'")
    picked = [c for c in cases if c.exp is not None and c.err is None and not any(k in c.name for k in skip)]
    ids = []
    for i, c in enumerate(picked):
        p = work / f"c{i}.heic"
        p.write_bytes(c.data)
        ids.append(wic.decode(p))
    wic.run()
    got, worst = 0, (99.0, "")
    for c, i in zip(picked, ids):
        r = wic.result(i)
        if r is None:
            s.notes.append(f"WIC could not decode: {c.name}")
            continue
        got += 1
        px = r[3]
        exp = wcases[c.name].exp
        if px.shape != exp.shape:
            s.fail(f"{c.name}: WIC size {px.shape} vs {exp.shape}")
            continue
        q = it.psnr(px[..., :3], exp[..., :3])
        worst = min(worst, (q, c.name))
        s.check(q >= 22, f"{c.name}: PSNR {q:.1f} dB against WIC")
    if got == 0:
        s.notes = ["WIC cannot decode HEIC here (HEIF / HEVC extensions missing): skipped"]
    else:
        s.notes.append(f"WIC decoded {got} / {len(picked)} files; lowest PSNR {worst[0]:.1f} dB ({worst[1]})")
    suites.append(s)


def grid_file(cache, seed, w, h, tw, th, params=""):
    """A w x h 4:2:0 grid of tw x th tiles (encoded separately) and its expected pixels."""
    rng = np.random.default_rng(seed)
    cols, rows = -(-w // tw), -(-h // th)
    src = gen_planes(int(rng.integers(1 << 30)), cols * tw, rows * th, 1, 8)
    hf = Heif()
    gid = hf.item(b"grid", bytes([0, 0, rows - 1, cols - 1]) + struct.pack(">HH", w, h))
    tiles = []
    exp = np.zeros((h, w, 4), np.uint8)
    for r in range(rows):
        for c in range(cols):
            tp = [src[0][r * th:(r + 1) * th, c * tw:(c + 1) * tw]] +                  [p[r * th // 2:(r + 1) * th // 2, c * tw // 2:(c + 1) * tw // 2] for p in src[1:]]
            ts = make_stream(cache, tw, th, 1, 8, params=params, planes=tp)
            tid = hf.item(b"hvc1", coded_data(ts), hidden=True)
            hf.prop(hvcc(ts), tid, True)
            hf.prop(ispe(tw, th), tid)
            tiles.append(tid)
            rgb = model_single(ts, stream_cicp(ts))
            y1, x1 = min(h, (r + 1) * th), min(w, (c + 1) * tw)
            exp[r * th:y1, c * tw:x1] = rgb[:y1 - r * th, :x1 - c * tw]
    hf.ref(b"dimg", gid, tiles)
    hf.prop(ispe(w, h), gid)
    hf.prop(nclx(1, 13, 6, 0), gid)
    return hf.build(gid), exp


def suite_perf(runners, cache, suites):
    data, exp = grid_file(cache, 77, 4032, 3024, 512, 512, "crf=24")
    for t, R in runners.items():
        s = it.Suite("perf", t)
        res, out = R.one(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT))
        s.check(res == 0 and it.max_abs(it.parse_decex(out)["px"], exp) == 0, "12 MP grid: pixels differ")
        reps = 2
        res, out = R.one(OP_HEICTIME, reps | (STRAIGHT << 16), data)
        if res != 0:
            s.fail(f"perf: error {it.err_name(res)}")
        else:
            ms = struct.unpack("<Q", out)[0] / 1000.0 / reps
            s.notes.append(f"4032x3024 grid, 48 tiles: {ms:.0f} ms per decode ({len(data) // 1024} KB)")
        img = photo(np.random.default_rng(5), 4032, 3024)
        t0 = time.time()
        res, enc = R.one(it.OP_ENCODE, 0, imgopt(95) + it.surface_input(img, C.FMT_ARGB))
        dt = time.time() - t0
        s.check(res == 0, f"12 MP encode: {it.err_name(res)}")
        if res == 0:
            s.notes.append(f"4032x3024 encode (q 95, 4:2:0): {dt * 1000:.0f} ms incl. process start, "
                           f"{len(enc) // 1024} KB")
        suites.append(s)


# ================================================================== encoder

def henc_input(planes, w, h, ch, qp, flags, vui=1 | 13 << 8 | 6 << 16 | 1 << 24):
    return struct.pack("<6I", w, h, ch, qp, flags, vui) + b"".join(p.astype(np.uint8).tobytes() for p in planes)


def suite_henc(runners, suites, seed, no_ext):
    import hevc_enc_ref as E
    rng = random.Random(seed)
    specs = [(16, 16, 1, 30, 0), (48, 40, 1, 30, 0), (64, 64, 1, 22, 0), (40, 24, 3, 27, 0), (32, 32, 0, 35, 0),
             (72, 40, 1, 12, 0), (32, 24, 3, 0, 1), (24, 16, 0, 0, 1), (50, 34, 1, 26, 2), (66, 38, 3, 33, 0),
             (40, 40, 1, 51, 0), (40, 40, 1, 0, 0), (8, 8, 1, 20, 0), (18, 10, 1, 25, 0), (80, 48, 0, 8, 0)]
    specs += [(rng.randrange(4, 50) * 2, rng.randrange(4, 30) * 2, rng.choice((0, 1, 3)), rng.randrange(52),
               rng.choice((0, 0, 0, 1, 2))) for _ in range(6)]
    recs = []
    for i, (w, h, ch, qp, fl) in enumerate(specs):
        planes = gen_planes(seed * 31 + i, w, h, ch, 8)
        r = E.encode([p.astype(int).tolist() for p in planes], ch, qp=qp, lossless=bool(fl & 1),
                     deblock=not (fl & 2))
        exp = b"".join(struct.pack("<I", len(n)) + n for n in (r.vps, r.sps, r.pps, r.slice))
        recon = np.concatenate([np.asarray(p, np.uint8).ravel() for p in r.recon]).tobytes()

        def chk(res, out, exp=exp, recon=recon, ch=ch, w=w, h=h, fl=fl):
            if res != 0:
                return f"error {it.err_name(res)}"
            out = bytes(out)
            if out[:-12] != exp:
                n = min(len(out) - 12, len(exp))
                d = next((k for k in range(n) if out[k] != exp[k]), n)
                return f"differs from the model at byte {d} of {len(exp)}"
            if not no_ext and fl & 2:
                nals, p = [], 0
                for _ in range(4):
                    ln = struct.unpack_from("<I", out, p)[0]
                    nals.append(out[p + 4:p + 4 + ln])
                    p += 4 + ln
                pf = {0: "gray", 1: "yuv420p", 3: "yuv444p"}[ch]
                dec = run_ff(["-f", "hevc", "-i", "-", "-f", "rawvideo", "-pix_fmt", pf, "-"],
                             b"".join(b"\0\0\0\1" + n for n in nals))
                if dec != recon:
                    return "ffmpeg's decode differs from the reconstruction"
            return None
        recs.append(Rec(OP_HEVCENC, 0, henc_input(planes, w, h, ch, qp, fl), chk, f"{w}x{h} cf{ch} qp{qp} f{fl}"))
    for t, R in runners.items():
        st = it.Suite("henc", t)
        it.run_suite(R, st, recs)
        suites.append(st)


def photo(rng, w, h, alpha=False):
    yy, xx = np.mgrid[0:h, 0:w].astype(float)
    r = 128 + 100 * np.sin(xx / 17) * np.cos(yy / 23)
    g = 128 + 90 * np.cos(xx / 11 + yy / 31)
    b = 128 + 80 * np.sin((xx + yy) / 19)
    img = np.stack([b, g, r, np.full_like(r, 255)], -1) + rng.normal(0, 3, (h, w, 4))
    img = np.clip(img, 0, 255).astype(np.uint8)
    img[..., 3] = np.clip(xx * 300 / max(1, w - 1) - 20, 0, 255).astype(np.uint8) if alpha else 255
    return img


def imgopt(q=0, flags=0, chroma=0):
    return struct.pack("<12IQQ", C.IMGF_HEIC, q, flags, 0, 0, 0, 0, chroma, 0, 0, 0, 0, 0, 0)


def suite_enc(runners, suites, seed, out, no_ext):
    rng = np.random.default_rng(seed)
    cases = [("1x1", 1, 1, 95, 0, 0, False, 20), ("2x2 444", 2, 2, 95, 0, 444, False, 30),
             ("64x48", 64, 48, 95, 0, 0, False, 33), ("33x17 odd (1x1 grid)", 33, 17, 90, 0, 0, False, 32),
             ("512x512", 512, 512, 95, 0, 0, False, 33), ("513x200 grid", 513, 200, 85, 0, 0, False, 32),
             ("1100x600 grid", 1100, 600, 75, 0, 0, False, 30), ("120x90 444", 120, 90, 95, 0, 444, False, 40),
             ("120x90 422 as 420", 120, 90, 95, 0, 422, False, 33), ("96x64 alpha", 96, 64, 90, 0, 0, True, 33),
             ("700x300 alpha grid", 700, 300, 90, 0, 0, True, 33), ("q1", 64, 64, 1, 0, 0, False, 18),
             ("q100", 64, 64, 100, 0, 444, False, 42), ("lossless", 40, 30, 0, 1, 0, False, 99),
             ("lossless odd", 37, 21, 0, 1, 0, False, 99), ("lossless alpha grid", 600, 520, 0, 1, 0, True, 99)]
    recs, imgs = [], {}
    for name, w, h, q, ll, ch, alpha, lim in cases:
        img = photo(rng, w, h, alpha)
        imgs[name] = img

        def chk(res, o, img=img, ll=ll, lim=lim, alpha=alpha):
            if res != 0:
                return f"error {it.err_name(res)}"
            return None
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(q, ll, ch) + it.surface_input(img, C.FMT_ARGB), chk, name))
    for t, R in runners.items():
        st = it.Suite("enc", t)
        res = it.run_suite(R, st, recs)
        files = {}
        drecs = []
        for (name, w, h, q, ll, ch, alpha, lim), r in zip(cases, res):
            if r is None or r[0] != 0:
                continue
            files[name] = bytes(r[1])
            img = imgs[name]

            def dchk(res2, o, img=img, ll=ll, lim=lim, alpha=alpha):
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
                q_ = it.psnr(d["px"][..., :3], img[..., :3])
                return None if q_ >= lim else f"PSNR {q_:.1f} < {lim}"
            drecs.append(Rec(it.OP_DECEX, 0, it.decex_input(files[name], flags=STRAIGHT), dchk, "dec " + name))
        it.run_suite(R, st, drecs)
        # metadata round trip
        import png_check as P
        icc = P.fake_icc(700, seed)
        exif = it.tiff_bytes([{"entries": [(0x0112, 3, [6]), (0x010F, 2, b"heic_check\0")], "next": None}])
        xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/">heic writer</x:xmpmeta>'
        img = photo(rng, 80, 60)
        flags = C.IOF_EMBED_ICC | C.IOF_EMBED_EXIF | C.IOF_EMBED_XMP
        rr, enc = R.one(OP_ENCMETA, 0, P.encmeta_input(imgopt(90, flags), img, C.FMT_ARGB, icc=icc, exif=exif, xmp=xmp))
        if rr != 0:
            st.fail(f"meta encode: {it.err_name(rr)}")
        else:
            rr, o = R.one(it.OP_DECEX, 0, it.decex_input(bytes(enc), flags=STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP))
            d = it.parse_decex(o) if rr == 0 else None
            st.check(d is not None and d["icc"] == icc, "ICC round trip")
            st.check(d is not None and d["xmp"] == xmp, "XMP round trip")
            ok = d is not None and len(d["exif"]) == len(exif) and d["exif"] != exif and \
                d["meta"]["orient"] == 1
            st.check(ok, "EXIF round trip (orientation set to 1)")
        # WIC: same pixels (its colour conversion / chroma filter differ a little)
        if t == "win" and not no_ext:
            work = out / "heic_wic_enc"
            work.mkdir(parents=True, exist_ok=True)
            wic = it.Wic(work)
            ids = {}
            # WIC: no 4:4:4 identity matrix (lossless) and no pictures below 16 x 16
            for name, data in files.items():
                if "lossless" in name or name in ("1x1", "2x2 444"):
                    continue
                pth = work / (name.replace(" ", "_") + ".heic")
                pth.write_bytes(data)
                ids[name] = wic.decode(pth)
            wic.run()
            got = 0
            for name, i in ids.items():
                r = wic.result(i)
                if r is None:
                    st.fail(f"WIC cannot read {name}")
                    continue
                got += 1
                px = r[3]
                img = imgs[name]
                if px.shape != img.shape:
                    st.fail(f"WIC {name}: size {px.shape} vs {img.shape}")
                    continue
                q_ = it.psnr(px[..., :3], img[..., :3])
                lim = 22 if name == "q1" else 30
                st.check(q_ >= lim, f"WIC {name}: PSNR {q_:.1f} against the source")
            st.notes.append(f"WIC read {got} / {len(ids)} written files")
        suites.append(st)


# ================================================================== main

def main():
    ap = it.argparser(__doc__, "hevc,heif,meta,errors,fuzz,henc,enc,perf,wic")
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(a.only.split(",")) if a.only else {"hevc", "heif", "meta", "errors", "fuzz", "henc", "enc", "perf",
                                                  "wic"}
    runners = {t: it.Runner(t, out, work_name="heic") for t in targets}
    cache = out / "heic_cache"
    suites = []
    cases, S = build_cases(cache, a.seed)
    if "hevc" in only:
        suite_hevc(runners, cache, suites)
    if "heif" in only:
        suite_cases(runners, cases, suites, "heif")
    if "meta" in only:
        suite_cases(runners, meta_cases(S), suites, "meta")
    if "errors" in only:
        suite_cases(runners, error_cases(S), suites, "errors")
    if "fuzz" in only:
        suite_fuzz(runners, cases, suites, a.fuzz, a.seed)
    if "henc" in only:
        suite_henc(runners, suites, a.seed, a.no_ext)
    if "enc" in only:
        suite_enc(runners, suites, a.seed, out, a.no_ext)
    if "perf" in only:
        suite_perf(runners, cache, suites)
    if "wic" in only and not a.no_ext and "win" in targets:
        suite_wic(cases, cache, a.seed, out, suites)
    ok = it.report(suites, "heic_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
