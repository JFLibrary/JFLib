#!/usr/bin/env python3
"""PNG checks for the CA core-codec package (img/png.s, img/imgcore.s, img/quantize.s).

Runs apps/codectest batch ops (240 img_decode_ex, 252 img_encode, 260..263 in
apps/codectest/src/png_ops.s) on every target and compares with Python references:

  straight  every colour type / bit depth / interlace / tRNS combination decoded with
            JDF_STRAIGHT (op 262) and IMGD_STRAIGHT (op 240): FMT_ARGB holding exactly the
            file's straight values (16-bit samples round(v / 257), the colour kept at alpha
            0 and for colour-key pixels); the premultiplied output (op 262 without the flag,
            op 240 default) must be premul(straight output) exactly
  meta      iCCP (zlib; > 16 MB inflated is dropped), sRGB, gAMA, cHRM (IM_CHRM), cICP,
            pHYs (dpi / unit), eXIf (IM_ORIENT; applied by img_decode_ex unless
            IMGD_NO_ORIENT), iTXt XMP (plain / zlib), acTL / fcTL (frames, loops, delay)
            -> op 261 (whole IMGMETA) and op 240 (oriented pixels)
  apng      APNG files: the default image (IDAT) is decoded, fcTL / fdAT around it and
            an IDAT that is not part of the animation are ignored
  enc       png_encode_cfg (op 263) and img_encode (op 252) with bit depth auto / 32 / 24 /
            8, dither levels 0..8, thresholds, effort; XRGB / PARGB / ARGB / A8 input;
            files read back with the Python reader below (every CRC, PLTE + tRNS) and a
            subset with WIC; ARGB at 32 bit is lossless (colour at alpha 0 included);
            auto is lossless; classic png_encode output is byte-identical to the C0
            reference binary (build/ca/ref, when present)
  encmeta   op 260: iCCP (zlib of the profile), pHYs from the dpi, eXIf with orientation
            reset to 1, iTXt XMP -- parsed back in Python
  fuzz      mutated PNGs with metadata chunks through op 240 (straight and premultiplied)
            and op 261: an error code or a well-formed result, never a crash (inputs end
            at a guard page)
  shots     8-bit palette encodes of a test photo (dither 0 / 4 / 8, threshold) written to
            <out>/pngcheck/*.png for a visual check (view them)

  python tools/png_check.py --target both --out build/ca
  python tools/png_check.py --target win --only straight,enc --no-build
"""

import random
import struct
import sys
import zlib
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
from codec_check import CHANS, PNG_TYPES, PNG_SIZES, png_make, rand_samples  # noqa: E402

C = it.C
OP_ENCMETA, OP_METAFULL, OP_PNGDECFL, OP_PNGENCCFG = 260, 261, 262, 263
PNG_SIG = b"\x89PNG\r\n\x1a\n"
SUITES = "straight,meta,apng,enc,encmeta,fuzz,shots"


# ================================================================ references

def to8(v, depth):
    v = np.asarray(v, np.int64)
    if depth == 16:
        return (v * 255 + 32895) >> 16
    if depth < 8:
        return v * 255 // ((1 << depth) - 1)
    return v


def expect_straight(samples, ctype, depth, plte=None, trns=None):
    """-> (h, w, 4) BGRA straight, exactly what the decoder must produce with JDF_STRAIGHT."""
    h, w, _ = samples.shape
    s = samples.astype(np.int64)
    a = np.full((h, w), 255, np.int64)
    if ctype == 0:
        g = to8(s[..., 0], depth)
        if trns is not None and len(trns) >= 2:
            a[s[..., 0] == struct.unpack(">H", trns[:2])[0]] = 0
        r = gg = b = g
    elif ctype == 2:
        if trns is not None and len(trns) >= 6:
            kr, kg, kb = struct.unpack(">HHH", trns[:6])
            a[(s[..., 0] == kr) & (s[..., 1] == kg) & (s[..., 2] == kb)] = 0
        r, gg, b = (to8(s[..., k], depth) for k in range(3))
    elif ctype == 3:
        pal = np.zeros((256, 4), np.int64)
        pal[:, 3] = 255
        p = np.asarray(plte, np.int64).reshape(-1, 3)
        pal[:len(p), :3] = p
        if trns is not None:
            t = np.frombuffer(trns, np.uint8)[:len(p)]
            pal[:len(t), 3] = t
        e = pal[s[..., 0]]
        r, gg, b, a = e[..., 0], e[..., 1], e[..., 2], e[..., 3]
    elif ctype == 4:
        r = gg = b = to8(s[..., 0], depth)
        a = to8(s[..., 1], depth)
    else:
        r, gg, b, a = (to8(s[..., k], depth) for k in range(4))
    return np.stack([b, gg, r, a], -1).astype(np.uint8)


def chunks_of(data):
    """-> list of (type, payload); raises on a bad CRC / structure."""
    if data[:8] != PNG_SIG:
        raise ValueError("signature")
    p, out = 8, []
    while p + 12 <= len(data):
        n = struct.unpack_from(">I", data, p)[0]
        t = bytes(data[p + 4:p + 8])
        d = bytes(data[p + 8:p + 8 + n])
        if p + 12 + n > len(data):
            raise ValueError("truncated chunk")
        if struct.unpack_from(">I", data, p + 8 + n)[0] != zlib.crc32(t + d) & 0xFFFFFFFF:
            raise ValueError(f"CRC of {t}")
        out.append((t, d))
        p += 12 + n
        if t == b"IEND":
            break
    if p != len(data) or not out or out[-1][0] != b"IEND":
        raise ValueError("IEND / trailing data")
    return out


def png_read(data):
    """Reader for the encoder's output (8 bit, colour types 0 / 2 / 3 / 6, no interlace).
    -> dict(ctype, w, h, bgra (straight, h x w x 4), npal, ntrns, chunks)."""
    ch = chunks_of(data)
    if ch[0][0] != b"IHDR":
        raise ValueError("IHDR first")
    w, h, depth, ctype, comp, flt, il = struct.unpack(">IIBBBBB", ch[0][1])
    if depth != 8 or il or comp or flt or ctype not in (0, 2, 3, 6):
        raise ValueError(f"IHDR {w} {h} {depth} {ctype} {il}")
    order = [t for t, _ in ch]
    plte = [d for t, d in ch if t == b"PLTE"]
    trns = [d for t, d in ch if t == b"tRNS"]
    if ctype == 3:
        if len(plte) != 1 or len(plte[0]) % 3 or not 3 <= len(plte[0]) <= 768:
            raise ValueError("PLTE")
        if order.index(b"PLTE") > order.index(b"IDAT"):
            raise ValueError("PLTE after IDAT")
        if trns and (len(trns[0]) > len(plte[0]) // 3 or order.index(b"tRNS") < order.index(b"PLTE")):
            raise ValueError("tRNS")
    idat = b"".join(d for t, d in ch if t == b"IDAT")
    if order.count(b"IDAT") and order.index(b"IDAT") + order.count(b"IDAT") != order.index(b"IEND"):
        raise ValueError("IDAT not consecutive before IEND")
    chans = CHANS[ctype]
    raw = zlib.decompress(idat)
    rb = w * chans
    if len(raw) != h * (rb + 1):
        raise ValueError("raw size")
    rows = np.frombuffer(raw, np.uint8).reshape(h, rb + 1)
    out = np.zeros((h, rb), np.int64)
    prior = np.zeros(rb, np.int64)
    for y in range(h):
        ft = rows[y, 0]
        x = rows[y, 1:].astype(np.int64)
        if ft > 4:
            raise ValueError(f"filter {ft}")
        if ft == 0:
            cur = x
        elif ft == 2:
            cur = (x + prior) & 255
        else:
            cur = np.zeros(rb, np.int64)
            for i in range(rb):
                a = cur[i - chans] if i >= chans else 0
                b = prior[i]
                c = prior[i - chans] if i >= chans else 0
                if ft == 1:
                    pr = a
                elif ft == 3:
                    pr = (a + b) >> 1
                else:
                    pp = a + b - c
                    pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                    pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                cur[i] = (x[i] + pr) & 255
        out[y] = cur
        prior = cur
    s = out.reshape(h, w, chans)
    npal = ntr = 0
    if ctype == 0:
        bgra = np.stack([s[..., 0]] * 3 + [np.full((h, w), 255)], -1)
    elif ctype == 2:
        bgra = np.stack([s[..., 2], s[..., 1], s[..., 0], np.full((h, w), 255)], -1)
    elif ctype == 6:
        bgra = np.stack([s[..., 2], s[..., 1], s[..., 0], s[..., 3]], -1)
    else:
        p = np.frombuffer(plte[0], np.uint8).reshape(-1, 3).astype(np.int64)
        npal = len(p)
        al = np.full(npal, 255, np.int64)
        if trns:
            t = np.frombuffer(trns[0], np.uint8)
            ntr = len(t)
            al[:ntr] = t
        idx = s[..., 0]
        if idx.max() >= npal:
            raise ValueError(f"index {idx.max()} >= palette {npal}")
        bgra = np.stack([p[idx, 2], p[idx, 1], p[idx, 0], al[idx]], -1)
    return dict(ctype=ctype, w=w, h=h, bgra=bgra.astype(np.uint8), npal=npal, ntrns=ntr, chunks=ch)


def over_white(bgra_straight):
    pm = it.premul(bgra_straight).astype(np.int64)
    a = pm[..., 3:4]
    return np.concatenate([pm[..., :3] + 255 - a, np.full_like(a, 255)], -1).astype(np.uint8)


# ================================================================ records

def meta_full(out):
    """op 261 result -> dict of IMGMETA fields + blobs."""
    b = bytes(out)
    m = {}
    m["fmt"], m["w"], m["h"] = struct.unpack_from("<III", b, 0)
    m["bits"], m["model"], m["orient"], m["channels"] = struct.unpack_from("<4B", b, 12)
    (m["flags"], m["frames"], m["dpix"], m["dpiy"], m["cicp"], m["gamma"]) = struct.unpack_from("<6I", b, 16)
    icclen, exiflen, xmplen = struct.unpack_from("<Q", b, 48)[0], struct.unpack_from("<Q", b, 64)[0], \
        struct.unpack_from("<Q", b, 80)[0]
    m["delay"], m["loops"], m["quality"], m["resunit"] = struct.unpack_from("<4I", b, 88)
    m["chrm"] = list(struct.unpack_from("<8H", b, 104))
    p = 128
    m["icc"] = b[p:p + icclen]
    p += icclen
    m["exif"] = b[p:p + exiflen]
    p += exiflen
    m["xmp"] = b[p:p + xmplen]
    p += xmplen
    if p != len(b):
        raise ValueError("op 261 length")
    return m


def expect_surface(exp, fmt):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        w, h, f, fl, px = it.parse_surface(out)
        if f != fmt:
            return f"fmt {f} != {fmt}"
        if px.shape != exp.shape:
            return f"shape {px.shape} != {exp.shape}"
        if not np.array_equal(px, exp):
            d = np.argwhere(np.any(px != exp, -1))
            y, x = d[0]
            return f"{len(d)} px differ, first ({x},{y}) got {list(px[y, x])} exp {list(exp[y, x])}"
        return None
    return chk


def imgopt(fmt, quality=0, flags=0, bitdepth=0, dither=0, athresh=0, chroma=0, effort=0):
    return struct.pack("<12IQQ", fmt, quality, flags, bitdepth, 0, dither, athresh, chroma, effort,
                       0, 0, 0, 0, 0)


def encmeta_input(opt, px, sfmt, dpix=0, dpiy=0, resunit=0, icc=b"", exif=b"", xmp=b""):
    return (opt + struct.pack("<6I", dpix, dpiy, resunit, len(icc), len(exif), len(xmp)) + icc + exif +
            xmp + it.surface_input(px, sfmt))


def fake_icc(n=600, seed=1):
    r = random.Random(seed)
    body = bytearray(r.randrange(256) for _ in range(n))
    body[0:4] = struct.pack(">I", n)
    body[36:40] = b"acsp"
    return bytes(body)


# ================================================================ suites

def suite_straight(rng):
    recs = []
    for ctype, depth in PNG_TYPES:
        ch = CHANS[ctype]
        for (w, h) in PNG_SIZES[:7] + [(33, 8)]:
            for il in (0, 1):
                s = rand_samples(rng, h, w, ch, depth, (w * h) % 2 == 0)
                plte = trns = None
                if ctype == 3:
                    npal = 1 << depth
                    plte = [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(npal)]
                    trns = bytes(rng.randrange(256) for _ in range(rng.randint(0, npal))) or None
                elif ctype in (0, 2) and (w * h + il) % 2 == 0:
                    trns = b"".join(struct.pack(">H", int(x)) for x in s[0, 0][:ch])
                if ctype in (4, 6):                     # force some alpha 0 with a colour
                    s[0, 0, -1] = 0
                data = png_make(s, ctype, depth, il, plte, trns, "random", rng=rng)
                st = expect_straight(s, ctype, depth, plte, trns)
                pm = it.premul(st)
                nm = f"ct{ctype} d{depth} {w}x{h} il{il}{' trns' if trns else ''}"
                recs.append(it.Rec(OP_PNGDECFL, C.JDF_STRAIGHT, data, expect_surface(st, C.FMT_ARGB), "fl straight " + nm))
                recs.append(it.Rec(OP_PNGDECFL, 0, data, expect_surface(pm, C.FMT_PARGB), "fl premul " + nm))
                recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_STRAIGHT),
                                   it.expect_decex(st, fmt=C.FMT_ARGB), "decex straight " + nm))
                recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data), it.expect_decex(pm, fmt=C.FMT_PARGB),
                                   "decex premul " + nm))
    # every 16-bit value once: round(v / 257)
    v = np.arange(65536, dtype=np.int64).reshape(256, 256, 1)
    data = png_make(v, 0, 16)
    recs.append(it.Rec(OP_PNGDECFL, C.JDF_STRAIGHT, data, expect_surface(expect_straight(v, 0, 16), C.FMT_ARGB),
                       "grey16 all values"))
    ga = np.concatenate([v, 65535 - v], -1)
    data = png_make(ga, 4, 16)
    st = expect_straight(ga, 4, 16)
    recs.append(it.Rec(OP_PNGDECFL, C.JDF_STRAIGHT, data, expect_surface(st, C.FMT_ARGB), "ga16 all values"))
    recs.append(it.Rec(OP_PNGDECFL, 0, data, expect_surface(it.premul(st), C.FMT_PARGB), "ga16 all values premul"))
    # every (colour, alpha) pair of RGBA8: straight exact, premultiplied = premul
    c = np.arange(256, dtype=np.int64)
    rgba = np.stack(np.meshgrid(c, c, indexing="ij"), -1)            # (256, 256, 2): colour, alpha
    s4 = np.stack([rgba[..., 0], 255 - rgba[..., 0], rgba[..., 0] // 2, rgba[..., 1]], -1)
    data = png_make(s4, 6, 8)
    st = expect_straight(s4, 6, 8)
    recs.append(it.Rec(OP_PNGDECFL, C.JDF_STRAIGHT, data, expect_surface(st, C.FMT_ARGB), "rgba8 all pairs"))
    recs.append(it.Rec(OP_PNGDECFL, 0, data, expect_surface(it.premul(st), C.FMT_PARGB), "rgba8 all pairs premul"))
    return recs


def phys(ppmx, ppmy, unit=1):
    return (b"pHYs", struct.pack(">IIB", ppmx, ppmy, unit))


def ppm_dpi(ppm):
    return min(0xFFFFFFFF, (ppm * 254 * 65536 + 5000) // 10000)


def suite_meta(rng):
    recs = []
    px = rand_samples(rng, 5, 7, 3, 8)
    icc = fake_icc(900, 3)
    exif = it.exif_blob(orient=6)
    xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/">' + b"x" * 300 + b"</x:xmpmeta>"
    chrm = [31270, 32900, 64000, 33000, 30000, 60000, 15000, 6000]
    extra = [
        (b"iCCP", b"My profile\0\0" + zlib.compress(icc, 9)),
        (b"sRGB", b"\0"),
        (b"gAMA", struct.pack(">I", 45455)),
        (b"cHRM", struct.pack(">8I", *chrm)),
        (b"cICP", bytes([1, 13, 0, 1])),
        phys(3780, 7559),
        (b"eXIf", exif),
        (b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0" + xmp),
    ]
    data = png_make(px, 2, 8, extra_chunks=extra)
    flags = C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP | C.IMGD_PROBE

    def chk_all(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        m = meta_full(out)
        exp = dict(fmt=C.IMGF_PNG, w=7, h=5, bits=8, model=C.IMM_RGB, orient=6, channels=3, gamma=45455,
                   cicp=0x01000D01, dpix=ppm_dpi(3780), dpiy=ppm_dpi(7559), resunit=C.IMR_CM)
        for k, v in exp.items():
            if m[k] != v:
                return f"{k} = {m[k]:#x}, expected {v:#x}"
        if not m["flags"] & C.IMF_SRGB or not m["flags"] & C.IMF_LOSSLESS:
            return f"flags {m['flags']:#x}"
        if m["chrm"] != [(v + 1) // 2 for v in chrm]:
            return f"chrm {m['chrm']}"
        if m["icc"] != icc:
            return f"icc {len(m['icc'])} bytes"
        if m["exif"] != exif:
            return "exif blob"
        if m["xmp"] != xmp:
            return f"xmp {m['xmp'][:40]!r}"
        return None
    recs.append(it.Rec(OP_METAFULL, flags, data, chk_all, "all chunks"))
    # blobs only when asked for
    def chk_noblob(res, out):
        m = meta_full(out)
        return None if res == 0 and not m["icc"] and not m["exif"] and not m["xmp"] and m["orient"] == 6 \
            else "blobs without flags"
    recs.append(it.Rec(OP_METAFULL, C.IMGD_PROBE, data, chk_noblob, "no blob flags"))
    # zlib-compressed iTXt XMP
    d2 = png_make(px, 2, 8, extra_chunks=[(b"iTXt", b"XML:com.adobe.xmp\0\1\0en\0\0" + zlib.compress(xmp))])
    recs.append(it.Rec(OP_METAFULL, C.IMGD_XMP | C.IMGD_PROBE, d2,
                       lambda r, o: None if r == 0 and meta_full(o)["xmp"] == xmp else "zlib XMP", "iTXt zlib"))
    # iCCP bomb: inflates to > 16 MB -> no profile, decode still fine
    bomb = zlib.compress(b"\0" * ((1 << 24) + 4096), 9)
    d3 = png_make(px, 2, 8, extra_chunks=[(b"iCCP", b"bomb\0\0" + bomb)])
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC, d3,
                       lambda r, o: None if r == 0 and meta_full(o)["icc"] == b"" else "bomb accepted", "iCCP > 16 MB"))
    # iCCP up to 16 MB accepted
    big = bytes(range(256)) * (1 << 14)                        # 4 MB
    d4 = png_make(px, 2, 8, extra_chunks=[(b"iCCP", b"big\0\0" + zlib.compress(big, 9))])
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC | C.IMGD_PROBE, d4,
                       lambda r, o: None if r == 0 and meta_full(o)["icc"] == big else "4 MB iCCP", "iCCP 4 MB"))
    # pHYs with unit 0 (aspect only) -> no dpi
    d5 = png_make(px, 2, 8, extra_chunks=[phys(2, 3, 0)])
    recs.append(it.Rec(OP_METAFULL, C.IMGD_PROBE, d5,
                       lambda r, o: None if meta_full(o)["dpix"] == 0 else "aspect pHYs gave dpi", "pHYs unit 0"))
    # orientation: applied by img_decode_ex, kept with IMGD_NO_ORIENT
    img = rand_samples(rng, 5, 9, 4, 8)
    img[..., 3] = np.where(img[..., 3] < 30, 0, img[..., 3])
    st = expect_straight(img, 6, 8)
    for o in range(1, 9):
        d = png_make(img, 6, 8, extra_chunks=[(b"eXIf", it.exif_blob(orient=o, bo=">" if o & 1 else "<"))])
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(d, flags=C.IMGD_STRAIGHT),
                           it.expect_decex(it.orient(st, o), fmt=C.FMT_ARGB, meta=dict(orient=o)), f"eXIf orient {o}"))
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(d, flags=C.IMGD_NO_ORIENT),
                           it.expect_decex(it.premul(st), fmt=C.FMT_PARGB, meta=dict(orient=o)), f"eXIf orient {o} kept"))
    # damaged / short metadata chunks are ignored
    bad = [(b"cHRM", b"\0" * 31), (b"gAMA", b"\0\0"), (b"pHYs", b"\0" * 8), (b"cICP", b"\1"),
           (b"iCCP", b"no nul" * 20), (b"iCCP", b"n\0\1xyz"), (b"iTXt", b"XML:com.adobe.xmp\0\1\0"),
           (b"eXIf", b"MM\0*\0\0\0\x08\0"), (b"acTL", b"\0\0\0\2")]
    d6 = png_make(px, 2, 8, extra_chunks=bad)
    recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP, d6,
                       lambda r, o: None if r == 0 else f"error {r}", "short chunks ignored"))
    recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(d6, flags=C.IMGD_STRAIGHT),
                       it.expect_decex(expect_straight(px, 2, 8)), "short chunks pixels"))
    return recs


def fctl(seq, w, h, x=0, y=0, num=1, den=10, dispose=0, blend=0):
    return (b"fcTL", struct.pack(">IIIIIHHBB", seq, w, h, x, y, num, den, dispose, blend))


def apng_file(rng, default_in_anim=True, frames=3, loops=0):
    w, h = 13, 9
    s = rand_samples(rng, h, w, 4, 8)
    raw = b"".join(b"\0" + bytes(np.asarray(r, np.uint8).reshape(-1)) for r in s)
    other = rand_samples(rng, h, w, 4, 8)
    oraw = b"".join(b"\0" + bytes(np.asarray(r, np.uint8).reshape(-1)) for r in other)
    out = [PNG_SIG, it.png_chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)),
           it.png_chunk(b"acTL", struct.pack(">II", frames, loops))]
    seq = 0
    if default_in_anim:
        out.append(it.png_chunk(*fctl(seq, w, h, num=7, den=100)))
        seq += 1
    out.append(it.png_chunk(b"IDAT", zlib.compress(raw)))
    for f in range(frames - (1 if default_in_anim else 0)):
        out.append(it.png_chunk(*fctl(seq, w - 2, h - 1, 1, 1, num=3, den=0)))
        seq += 1
        z = zlib.compress(oraw[: (h - 1) * (1 + (w - 2) * 4)] if False else oraw)
        out.append(it.png_chunk(b"fdAT", struct.pack(">I", seq) + z))
        seq += 1
    out.append(it.png_chunk(b"IEND", b""))
    return b"".join(out), s


def suite_apng(rng):
    recs = []
    for inanim in (True, False):
        for frames, loops in ((1, 0), (3, 0), (4, 7)):
            data, s = apng_file(rng, inanim, frames, loops)
            st = expect_straight(s, 6, 8)
            nm = f"apng frames {frames} loops {loops} default {'in' if inanim else 'outside'}"
            meta = dict(frames=frames, loops=loops)
            if inanim:
                meta["delay"] = 70
            if frames > 1:
                meta["flags"] = C.IMF_ANIMATED | C.IMF_LOSSLESS | C.IMF_ALPHA
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=C.IMGD_STRAIGHT),
                               it.expect_decex(st, fmt=C.FMT_ARGB, meta=meta), nm))
            recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(data, frame=1), it.expect_err(C.ERR_UNSUPPORTED),
                               nm + " frame 1"))
    return recs


def smooth_photo(rng, w, h):
    r = np.random.default_rng(rng.randrange(1 << 30))
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    img = np.zeros((h, w, 3))
    for k in range(3):
        img[..., k] = 128 + 100 * np.sin(xx / (7 + 5 * k) + yy / (11 - 2 * k) + r.uniform(0, 6))
    img += r.normal(0, 3, img.shape)
    return np.clip(img, 0, 255).astype(np.uint8)


def src_image(rng, w, h, kind):
    """straight BGRA test image: 'few' (<= 200 colours incl. partial alpha), 'photo'
    (many colours, opaque), 'alpha' (many colours, all alphas), 'binary' (alpha 0 / 255)."""
    r = np.random.default_rng(rng.randrange(1 << 30))
    if kind == "few":
        pal = r.integers(0, 256, (200, 4))
        pal[:40, 3] = 255
        pal[40:45, 3] = 0
        return pal[r.integers(0, 200, (h, w))].astype(np.uint8)
    rgb = smooth_photo(rng, w, h)[..., ::-1]
    a = np.full((h, w, 1), 255)
    if kind == "alpha":
        a = r.integers(0, 256, (h, w, 1))
    elif kind == "binary":
        a = np.where(r.random((h, w, 1)) < 0.3, 0, 255)
    return np.concatenate([rgb, a], -1).astype(np.uint8)


def as_input(st, sfmt):
    """straight BGRA -> surface pixels of format sfmt (+ what the encoder sees as straight)."""
    if sfmt == C.FMT_ARGB:
        return st, st.copy()
    if sfmt == C.FMT_PARGB:
        pm = it.premul(st)
        return pm, it.unpremul(pm)
    if sfmt == C.FMT_XRGB:
        x = st.copy()
        x[..., 3] = np.where(x[..., 3] == 255, 7, x[..., 3])          # garbage alpha byte
        seen = st.copy()
        seen[..., 3] = 255
        return x, seen
    g = st[..., 1]
    seen = np.stack([g, g, g, np.full_like(g, 255)], -1)
    return g, seen


def expect_png(check):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        try:
            d = png_read(out)
        except Exception as e:  # noqa: BLE001
            return f"invalid PNG: {e}"
        return check(d)
    return chk


def chk_exact(exp, ctype=None, alpha0_zero=False):
    exp = np.asarray(exp, np.uint8)
    if alpha0_zero:
        exp = exp.copy()
        exp[exp[..., 3] == 0] = 0

    def c(d):
        if ctype is not None and d["ctype"] != ctype:
            return f"colour type {d['ctype']} != {ctype}"
        got = d["bgra"].copy()
        if alpha0_zero:
            got[got[..., 3] == 0] = 0
        if got.shape != exp.shape:
            return f"shape {got.shape} != {exp.shape}"
        if not np.array_equal(got, exp):
            y, x = np.argwhere(np.any(got != exp, -1))[0]
            return f"pixel ({x},{y}) {list(got[y, x])} != {list(exp[y, x])}"
        return None
    return c


def chk_palette(seen, thresh, min_psnr, exact=False):
    """8-bit palette output: alpha < thresh -> transparent, else opaque; colours within PSNR."""
    def c(d):
        if d["ctype"] != 3:
            return f"colour type {d['ctype']}"
        got = d["bgra"].astype(np.int64)
        tr = seen[..., 3] < thresh
        if np.any(got[tr][:, 3] != 0) or np.any(got[~tr][:, 3] != 255):
            return "alpha not binary by the threshold"
        if d["npal"] > 256:
            return "palette > 256"
        if (~tr).any():
            p = it.psnr(got[~tr][:, :3], seen[~tr][:, :3])
            if exact and p < 99:
                return f"not exact (PSNR {p:.1f})"
            if p < min_psnr:
                return f"PSNR {p:.2f} < {min_psnr}"
        return None
    return c


def suite_enc(rng, out):
    recs = []
    sizes = [(1, 1), (3, 2), (17, 5), (64, 33)]
    for (w, h) in sizes:
        for kind in ("few", "photo", "alpha", "binary"):
            st = src_image(rng, w, h, kind)
            for sfmt in (C.FMT_XRGB, C.FMT_PARGB, C.FMT_ARGB, C.FMT_A8):
                px, seen = as_input(st, sfmt)
                sin = it.surface_input(px, sfmt)
                nm = f"{w}x{h} {kind} fmt{sfmt}"
                opaque = bool(np.all(seen[..., 3] == 255))
                # 32 bit: straight RGBA exactly (ARGB keeps colour at alpha 0)
                recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 32, 0, 0) + sin,
                                   expect_png(chk_exact(seen, 6, alpha0_zero=sfmt != C.FMT_ARGB)), "32 " + nm))
                # 24 bit: over white
                recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 24, 0, 0) + sin,
                                   expect_png(chk_exact(over_white(seen), 2)), "24 " + nm))
                # auto: lossless (alpha-0 colours may be dropped); colour type rule
                ncol = len(np.unique(np.where(seen[..., 3:4] == 0, 0, seen).reshape(-1, 4).view(np.uint32)))

                def auto_chk(d, seen=seen, ncol=ncol, opaque=opaque, sfmt=sfmt):
                    want = 0 if sfmt == C.FMT_A8 else (3 if ncol <= 256 else (2 if opaque else 6))
                    if d["ctype"] != want:
                        return f"auto colour type {d['ctype']} != {want} ({ncol} colours)"
                    return chk_exact(seen, None, alpha0_zero=True)(d)
                recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 0, 0, 0) + sin, expect_png(auto_chk),
                                   "auto " + nm))
                # 8 bit with thresholds and dither levels
                for level, thr in ((0, 128), (8, 128), (4, 1), (7, 255), (8, 0)):
                    qcfg = level | thr << 8
                    ncol_b = len(np.unique(np.where(seen[..., 3:4] < thr, 0, seen | np.array([0, 0, 0, 255], np.uint8))
                                           .reshape(-1, 4).view(np.uint32)))
                    exact = ncol_b <= 256
                    recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 8, qcfg, 0) + sin,
                                       expect_png(chk_palette(seen, thr, 99 if exact else 22, exact)),
                                       f"8 L{level} T{thr} " + nm))
                # img_encode path (op 252): biased IO_DITHER / IO_ATHRESH, effort
                opt = imgopt(C.IMGF_PNG, bitdepth=8, dither=1 + 3, athresh=1 + 200, effort=1)
                recs.append(it.Rec(it.OP_ENCODE, 0, opt + sin, expect_png(chk_palette(seen, 200, 20)),
                                   "img_encode 8 " + nm))
                opt = imgopt(C.IMGF_PNG, bitdepth=32, effort=9)
                recs.append(it.Rec(it.OP_ENCODE, 0, opt + sin,
                                   expect_png(chk_exact(seen, 6, alpha0_zero=sfmt != C.FMT_ARGB)), "img_encode 32 " + nm))
                opt = imgopt(C.IMGF_PNG)
                recs.append(it.Rec(it.OP_ENCODE, 0, opt + sin, expect_png(chk_exact(seen, None, alpha0_zero=True)),
                                   "img_encode auto " + nm))
    # errors
    recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 32, 0, 0) + struct.pack("<III", 2, 2, 7) + bytes(16),
                       it.expect_any(it.expect_err(-100), it.expect_err(C.ERR_UNSUPPORTED)), "surface format 7"))
    return recs


def classic_identity(runners, out, rng, suite):
    """png_encode (op 8) / png_decode (op 6) of the new build vs the C0 reference binary."""
    ref = it.ROOT / "build" / "ca" / "ref" / "codectest_c0.exe"
    if "win" not in runners or not ref.exists():
        suite.notes.append("classic identity: reference binary missing, skipped")
        return
    rr = it.Runner("win", out, work_name="png_ref")
    rr.exe = ref
    recs = []
    for (w, h) in ((1, 1), (5, 3), (40, 17), (100, 61)):
        for kind in ("few", "photo", "alpha", "binary"):
            st = src_image(rng, w, h, kind)
            for sfmt in (C.FMT_XRGB, C.FMT_PARGB, C.FMT_A8):
                px, _ = as_input(st, sfmt)
                recs.append(it.Rec(8, 0, it.surface_input(px, sfmt), None, f"classic {w}x{h} {kind} {sfmt}"))
    new = runners["win"].batch(recs)
    old = rr.batch(recs)
    for r, a, b in zip(recs, new, old):
        suite.check(a is not None and b is not None and a == b, f"png_encode bytes differ: {r.name}")


def suite_encmeta(rng):
    recs = []
    st = src_image(rng, 23, 11, "alpha")
    icc = fake_icc(1500, 7)
    exif = it.exif_blob(orient=8, xres=(300, 1), yres=(300, 1), unit=2)
    xmp = b"<?xpacket begin?><x:xmpmeta>" + b"y" * 200 + b"</x:xmpmeta><?xpacket end='w'?>"
    for flags in (0, C.IOF_EMBED_ICC, C.IOF_EMBED_EXIF, C.IOF_EMBED_XMP,
                  C.IOF_EMBED_ICC | C.IOF_EMBED_EXIF | C.IOF_EMBED_XMP):
        for depth in (0, 8, 32):
            opt = imgopt(C.IMGF_PNG, flags=flags, bitdepth=depth)
            dpi = 300 << 16

            def chk(d, flags=flags):
                ch = d["chunks"]
                names = [t for t, _ in ch]
                ph = [p for t, p in ch if t == b"pHYs"]
                if len(ph) != 1 or struct.unpack(">IIB", ph[0]) != (11811, 11811, 1):
                    return f"pHYs {ph}"
                ic = [p for t, p in ch if t == b"iCCP"]
                if bool(flags & C.IOF_EMBED_ICC) != bool(ic):
                    return "iCCP presence"
                if ic:
                    nul = ic[0].index(b"\0")
                    if ic[0][nul + 1] != 0 or zlib.decompress(ic[0][nul + 2:]) != icc:
                        return "iCCP content"
                ex = [p for t, p in ch if t == b"eXIf"]
                if bool(flags & C.IOF_EMBED_EXIF) != bool(ex):
                    return "eXIf presence"
                if ex:
                    ex1 = it.exif_blob(orient=1, xres=(300, 1), yres=(300, 1), unit=2)
                    if ex[0] != ex1:
                        return "eXIf content (orientation 1 expected)"
                xt = [p for t, p in ch if t == b"iTXt"]
                if bool(flags & C.IOF_EMBED_XMP) != bool(xt):
                    return "iTXt presence"
                if xt and xt[0] != b"XML:com.adobe.xmp\0\0\0\0\0" + xmp:
                    return "iTXt content"
                for t in (b"iCCP", b"pHYs", b"eXIf"):
                    if t in names and names.index(t) > names.index(b"IDAT"):
                        return f"{t} after IDAT"
                if b"PLTE" in names and b"iCCP" in names and names.index(b"iCCP") > names.index(b"PLTE"):
                    return "iCCP after PLTE"
                return None
            recs.append(it.Rec(OP_ENCMETA, 0, encmeta_input(opt, st, C.FMT_ARGB, dpi, dpi, C.IMR_INCH, icc, exif, xmp),
                               expect_png(chk), f"meta flags {flags:#x} depth {depth}"))
    # round trip through our decoder: the metadata comes back
    opt = imgopt(C.IMGF_PNG, flags=C.IOF_EMBED_ICC | C.IOF_EMBED_EXIF | C.IOF_EMBED_XMP)
    return recs, encmeta_input(opt, st, C.FMT_ARGB, 72 << 16, 96 << 16, C.IMR_INCH, icc, exif, xmp), (icc, exif, xmp)


def suite_fuzz(rng, count):
    bases = []
    px = rand_samples(rng, 9, 12, 4, 8)
    ex = [(b"iCCP", b"p\0\0" + zlib.compress(fake_icc(300))), (b"sRGB", b"\0"), (b"gAMA", b"\0\0\xb1\x8f"),
          (b"cHRM", bytes(32)), (b"cICP", b"\1\1\0\1"), phys(100, 200), (b"eXIf", it.exif_blob(orient=3)),
          (b"iTXt", b"XML:com.adobe.xmp\0\1\0\0\0" + zlib.compress(b"<xmp/>" * 20)), (b"acTL", b"\0\0\0\2\0\0\0\0"),
          fctl(0, 12, 9)]
    bases.append(png_make(px, 6, 8, extra_chunks=ex))
    bases.append(png_make(rand_samples(rng, 7, 5, 1, 4), 3, 4, plte=[(i, i, i) for i in range(16)],
                          trns=bytes(range(0, 160, 10)), extra_chunks=ex[:3]))
    bases.append(png_make(rand_samples(rng, 6, 6, 2, 16), 4, 16, interlace=1, extra_chunks=ex[5:]))
    bases.append(apng_file(rng, True, 3, 1)[0])
    recs = []
    for i in range(count):
        b = bases[i % len(bases)]
        if i % 3 == 0:
            m = it.mutate(rng, b, keep=8)
        else:
            # mutate a chunk payload and fix its CRC so the parser gets past the check
            ch = chunks_of(b)
            j = rng.randrange(1, len(ch) - 1)
            t, d = ch[j]
            d = it.mutate(rng, d) if d else bytes([rng.randrange(256)])
            ch[j] = (t, d)
            m = PNG_SIG + b"".join(it.png_chunk(t, d) for t, d in ch)
        flags = [C.IMGD_STRAIGHT, 0, C.IMGD_STRAIGHT | C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP][i % 3]
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(m, flags=flags), None, f"fuzz#{i}"))
        if i % 4 == 0:
            recs.append(it.Rec(OP_METAFULL, C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP | C.IMGD_PROBE, m, None,
                               f"fuzz#{i} meta"))
    return recs


def fuzz_check(res, out):
    if res == 0 or C.ERR_LIMIT <= res <= -1:
        return None
    return f"bad result {res}"


def write_shots(runner, rng, outdir):
    outdir.mkdir(parents=True, exist_ok=True)
    w, h = 192, 128
    st = src_image(rng, w, h, "photo")
    yy, xx = np.mgrid[0:h, 0:w]
    st[..., 3] = np.clip(255 - np.hypot(xx - w / 2, yy - h / 2) * 3, 0, 255).astype(np.uint8)
    files = []
    recs = []
    for level, thr in ((0, 128), (4, 128), (8, 128), (8, 1)):
        recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 8, level | thr << 8, 0) +
                           it.surface_input(st, C.FMT_ARGB), None, f"shot L{level} T{thr}"))
        files.append(outdir / f"pal8_dither{level}_t{thr}.png")
    recs.append(it.Rec(OP_PNGENCCFG, 0, struct.pack("<4I", 6, 32, 0, 0) + it.surface_input(st, C.FMT_ARGB), None,
                       "shot 32"))
    files.append(outdir / "rgba32.png")
    res = runner.batch(recs)
    for f, r in zip(files, res):
        if r is not None and r[0] == 0:
            f.write_bytes(r[1])
    return files


def main():
    ap = it.argparser(__doc__, SUITES)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        it.build_codectest(targets, out)
    rng = random.Random(a.seed)
    runners = {t: it.Runner(t, out, work_name="pngcheck") for t in targets}
    suites = []
    gen = []
    if want("straight"):
        gen.append(("straight", suite_straight(random.Random(a.seed + 1))))
    if want("meta"):
        gen.append(("meta", suite_meta(random.Random(a.seed + 2))))
    if want("apng"):
        gen.append(("apng", suite_apng(random.Random(a.seed + 3))))
    if want("enc"):
        gen.append(("enc", suite_enc(random.Random(a.seed + 4), out)))
    meta_rt = None
    if want("encmeta"):
        recs, meta_rt, blobs = suite_encmeta(random.Random(a.seed + 5))
        gen.append(("encmeta", recs))
    if want("fuzz") and a.fuzz:
        recs = suite_fuzz(random.Random(a.seed + 6), a.fuzz)
        for r in recs:
            r.check = fuzz_check
        gen.append(("fuzz", recs))
    for t, runner in runners.items():
        for name, recs in gen:
            s = it.Suite(name, t)
            it.run_suite(runner, s, recs)
            suites.append(s)
        if meta_rt is not None:
            s = it.Suite("metart", t)
            res, png = runner.one(OP_ENCMETA, 0, meta_rt)
            s.check(res == 0, f"encode error {res}")
            if res == 0:
                r2, o2 = runner.one(OP_METAFULL, C.IMGD_ICC | C.IMGD_EXIF | C.IMGD_XMP | C.IMGD_PROBE, png)
                m = meta_full(o2) if r2 == 0 else None
                s.check(m is not None and m["icc"] == blobs[0], "ICC round trip")
                s.check(m is not None and m["exif"] == it.exif_blob(orient=1, xres=(300, 1), yres=(300, 1), unit=2),
                        "EXIF round trip (orientation 1)")
                s.check(m is not None and m["xmp"] == blobs[2], "XMP round trip")
                s.check(m is not None and (m["dpix"] >> 16, m["dpiy"] >> 16) in ((71, 95), (72, 96)),
                        f"dpi round trip {m and (m['dpix'] / 65536, m['dpiy'] / 65536)}")
            suites.append(s)
        if want("enc") and t == "win":
            s = it.Suite("classic", t)
            classic_identity(runners, out, random.Random(a.seed + 7), s)
            suites.append(s)
    if want("enc") and not a.no_ext and it.build.IS_WIN_HOST and "win" in runners:
        s = it.Suite("wic", "win")
        wic = it.Wic(out / "win" / "pngcheck" / "wic")
        jobs = []
        r2 = random.Random(a.seed + 8)
        for depth in (0, 8, 24, 32):
            for kind in ("few", "photo", "alpha", "binary"):
                st = src_image(r2, 37, 21, kind)
                res, data = runners["win"].one(OP_PNGENCCFG, 0, struct.pack("<4I", 6, depth, 8 | 128 << 8, 0) +
                                               it.surface_input(st, C.FMT_ARGB))
                if res != 0:
                    s.fail(f"encode {depth} {kind}: {res}")
                    continue
                f = out / "win" / "pngcheck" / f"wic_{depth}_{kind}.png"
                f.write_bytes(data)
                jobs.append((wic.decode(f), png_read(data)["bgra"], f"{depth} {kind}"))
        wic.run()
        for i, exp, nm in jobs:
            r = wic.result(i)
            if r is None:
                s.fail(f"WIC cannot decode {nm}")
                continue
            got = r[3].copy()
            e = exp.copy()
            got[got[..., 3] == 0] = 0
            e[e[..., 3] == 0] = 0
            s.check(np.array_equal(got, e), f"WIC decode differs: {nm} (max {it.max_abs(got, e)})")
        suites.append(s)
    if want("shots") and runners:
        t = "win" if "win" in runners else targets[0]
        s = it.Suite("shots", t)
        files = write_shots(runners[t], random.Random(a.seed + 9), out / t / "pngcheck")
        s.check(all(f.exists() for f in files), "shots not written")
        s.notes.append("view: " + ", ".join(it.Runner.rel(f) for f in files if f.exists()))
        suites.append(s)
    return 0 if it.report(suites, "png_check") else 1


if __name__ == "__main__":
    sys.exit(main())
