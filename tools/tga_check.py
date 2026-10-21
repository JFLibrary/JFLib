#!/usr/bin/env python3
"""Checks of the TGA reader / writer (ASMLIB/src/img/tga.s) through apps/codectest
(op 240 img_decode_ex, op 241 img_probe, op 252 img_encode).

A Python TGA writer produces every variant the decoder accepts: image types 1 / 2 / 3 and
RLE 9 / 10 / 11, depths 8 / 15 / 16 / 24 / 32 (grey 8 / 16), colour maps with 15 / 16 / 24
/ 32-bit entries and a first-entry index (8- and 16-bit indices, indices outside the map),
ID fields, all four origins, RLE packets that cross rows (and ones that overrun the
image), v2 footers with extension areas (attributes type 0..4, bad offsets / sizes) and
16-bit / 32-bit alpha with and without descriptor alpha bits.  A Python reference decoder
implements the documented semantics (see tga.s); results must match exactly.

Suites
  decode    Python-written files -> img_decode_ex (IMGD_STRAIGHT and premultiplied) ==
            reference; IMGMETA (size, bits, model, channels, alpha / premultiplied flags)
  ffmpeg    `ffmpeg -c:v targa -rle 0|1 -pix_fmt bgra|bgr24|rgb555le|gray|pal8` files and
            the Python-written variants ffmpeg understands -> ours == ffmpeg's decode
            (not 15-bit right-to-left files: ffmpeg does not flip those rows -- its bug)
  encode    img_encode TGA 32 / 24 bit, RLE on / off, from ARGB / PARGB / XRGB / A8
            surfaces: header, extension area (attributes type), footer, RLE packets never
            crossing rows, size bound; decoded by ffmpeg and by us == the input (24 bit:
            premultiplied over white)
  detect    TGA with and without the extension hint (strict tga_probe), probe meta
  errors    truncations, unsupported / bad types, depths, interleave bits, dimensions,
            cancel, frame 1, pixel limit
  fuzz      mutated files (hint TGA) -> any result, no crash (inputs end at a guard page)

  python tools/tga_check.py --target both --out build/cd
"""

import random
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402

C = it.C
Rec = it.Rec
SUITES = "decode,ffmpeg,encode,detect,errors,fuzz"
SIG = b"TRUEVISION-XFILE.\0"


# ================================================================== writer

def enc16(bgra, abit):
    """(n, 4) BGRA 8-bit -> A1R5G5B5 u16 (5-bit channels: top 5 bits)."""
    b, g, r = (bgra[:, 0] >> 3).astype(np.uint16), (bgra[:, 1] >> 3).astype(np.uint16), (bgra[:, 2] >> 3).astype(np.uint16)
    return (b | (g << 5) | (r << 10) | (abit.astype(np.uint16) << 15)).astype(np.uint16)


def rle_encode(pix, bpp, rng, cross=True, w=None, maxrun=128):
    """Stream of stored pixels (bytes, bpp each) -> RLE packets; cross=False keeps packets
    inside rows of w pixels; random packet lengths exercise every case."""
    n = len(pix) // bpp
    out = bytearray()
    i = 0
    while i < n:
        lim = n - i
        if not cross:
            lim = min(lim, w - i % w)
        lim = min(lim, maxrun)
        px = pix[i * bpp:(i + 1) * bpp]
        run = 1
        while run < lim and pix[(i + run) * bpp:(i + run + 1) * bpp] == px:
            run += 1
        if run >= 2 and rng.random() < 0.9:
            k = rng.randint(2, run) if rng.random() < 0.3 else run
            out.append(0x80 | (k - 1))
            out += px
            i += k
        else:
            k = rng.randint(1, lim) if rng.random() < 0.5 else min(lim, rng.randint(1, 8))
            out.append(k - 1)
            out += pix[i * bpp:(i + k) * bpp]
            i += k
    return bytes(out)


def tga_bytes(w, h, itype, depth, stream, desc=0, cmap=None, idfield=b"", ext=None, footer=None,
              xo=0, yo=0):
    """stream = stored pixels in file order (already RLE-coded for types 9..11);
    cmap = (first, entries bytes, entry bits, count) or None; ext = attributes type
    (writes a v2 extension area + footer), footer = raw 26-byte footer override."""
    cmtype = 1 if cmap else 0
    first, cmdata, cbits, count = cmap if cmap else (0, b"", 0, 0)
    hdr = struct.pack("<BBBHHBHHHHBB", len(idfield), cmtype, itype, first, count, cbits, xo, yo, w, h, depth, desc)
    body = hdr + idfield + cmdata + stream
    if ext is not None:
        area = bytearray(495)
        struct.pack_into("<H", area, 0, 495)
        area[494] = ext & 0xFF
        off = len(body)
        body += bytes(area) + struct.pack("<II", off, 0) + SIG
    elif footer is not None:
        body += footer
    return body


# ================================================================== reference decoder

def exp5(v):
    v = v.astype(np.int64)
    return ((v << 3) | (v >> 2)).astype(np.uint8)


def ref_decode(data):
    """-> (err, img (h, w, 4) BGRA straight or None, meta dict, premul flag)."""
    E = C
    n = len(data)
    if n < 18:
        return E.ERR_TRUNC, None, None, False
    idlen, cmtype, itype = data[0], data[1], data[2]
    if itype in (0, 32, 33):
        return E.ERR_UNSUPPORTED, None, None, False
    base = itype & 0xF7
    if base not in (1, 2, 3):
        return E.ERR_HDR, None, None, False
    if cmtype > 1:
        return E.ERR_HDR, None, None, False
    first, cmlen, cmbits = struct.unpack_from("<HHB", data, 3)
    mapoff = 18 + idlen
    mapbytes = cmlen * ((cmbits + 7) // 8) if cmtype else 0
    pixoff = mapoff + mapbytes
    if pixoff > n:
        return E.ERR_TRUNC, None, None, False
    if base == 1 and (cmtype == 0 or cmlen == 0):
        return E.ERR_HDR, None, None, False
    depth, desc = data[16], data[17]
    if desc & 0xC0:
        return E.ERR_UNSUPPORTED, None, None, False
    storage = 0
    if base == 1:
        if depth not in (8, 16) or cmbits not in (15, 16, 24, 32):
            return E.ERR_UNSUPPORTED, None, None, False
        storage = {15: 0, 24: 0, 16: 2, 32: 1}[cmbits]
    elif base == 2:
        if depth not in (15, 16, 24, 32):
            return E.ERR_UNSUPPORTED, None, None, False
        storage = {15: 0, 16: 2, 24: 0, 32: 1}[depth]
    else:
        if depth not in (8, 16):
            return E.ERR_UNSUPPORTED, None, None, False
        storage = 3 if depth == 16 else 0
    w, h = struct.unpack_from("<HH", data, 12)
    bpp = (depth + 7) // 8
    # footer / extension area
    ext = None
    if n >= 18 + 26 and data[n - 18:] == SIG:
        off = struct.unpack_from("<I", data, n - 26)[0]
        if off >= 18 and off + 495 <= n - 26 and struct.unpack_from("<H", data, off)[0] >= 495:
            ext = data[off + 494]
    premul = heur = False
    if ext is not None:
        use = storage != 0 and ext in (3, 4)
        premul = use and ext == 4
    else:
        use = storage in (1, 3) or (storage == 2 and (desc & 15) != 0)
        heur = use
    a16 = use and storage == 2
    meta = dict(w=w, h=h, bits=5 if (base == 2 and depth in (15, 16)) or (base == 1 and cmbits <= 16) else 8,
                model={1: C.IMM_PAL, 2: C.IMM_RGB, 3: C.IMM_GRAY}[base],
                channels=(3 if base == 2 else 1) + int(use), alpha=use, premul=premul)
    if w == 0 or h == 0:
        return E.ERR_DIMS, None, meta, premul

    def conv(raw, kind):
        """stored bytes of m pixels -> (m, 4) BGRA."""
        m = len(raw) // {"t16": 2, "t24": 3, "t32": 4, "g8": 1, "g16": 2}[kind]
        a = np.frombuffer(raw, np.uint8)
        if kind == "t32":
            p = a.reshape(m, 4).copy()
            if not use:
                p[:, 3] = 255
            return p
        if kind == "t24":
            p = np.full((m, 4), 255, np.uint8)
            p[:, :3] = a.reshape(m, 3)
            return p
        if kind == "g8":
            return np.stack([a, a, a, np.full(m, 255, np.uint8)], -1)
        if kind == "g16":
            p = a.reshape(m, 2)
            return np.stack([p[:, 0], p[:, 0], p[:, 0], p[:, 1] if use else np.full(m, 255, np.uint8)], -1)
        v = np.frombuffer(raw, np.uint16)
        out = np.stack([exp5(v & 31), exp5((v >> 5) & 31), exp5((v >> 10) & 31),
                        np.where(v & 0x8000, 255, 0).astype(np.uint8) if a16 else np.full(m, 255, np.uint8)], -1)
        return out

    # palette
    pal = None
    if base == 1:
        nent = 1 << depth
        pal = np.zeros((nent, 4), np.uint8)
        pal[:, 3] = 255
        esz = (cmbits + 7) // 8
        kind = {15: "t16", 16: "t16", 24: "t24", 32: "t32"}[cmbits]
        ents = conv(bytes(data[mapoff:mapoff + cmlen * esz]), kind)
        k = min(cmlen, max(0, nent - first))
        pal[first:first + k] = ents[:k]
    total = w * h
    kind = {1: None, 2: {15: "t16", 16: "t16", 24: "t24", 32: "t32"}.get(depth),
            3: "g8" if depth == 8 else "g16"}[base]

    def px_of(raw):
        if base == 1:
            idx = np.frombuffer(raw, np.uint8 if depth == 8 else np.uint16)
            return pal[idx]
        return conv(raw, kind)
    if itype < 8:
        need = total * bpp
        if pixoff + need > n:
            return E.ERR_TRUNC, None, meta, premul
        stream = px_of(bytes(data[pixoff:pixoff + need]))
    else:
        out, pos, got = [], pixoff, 0
        while got < total:
            if pos >= n:
                return E.ERR_TRUNC, None, meta, premul
            b = data[pos]
            pos += 1
            cnt = (b & 0x7F) + 1
            if b & 0x80:
                if n - pos < bpp:
                    return E.ERR_TRUNC, None, meta, premul
                p = px_of(bytes(data[pos:pos + bpp]))
                pos += bpp
                k = min(cnt, total - got)
                out.append(np.repeat(p, k, 0))
                got += k
            else:
                k = min(cnt, total - got)
                for _ in range(k):
                    if n - pos < bpp:
                        return E.ERR_TRUNC, None, meta, premul
                    out.append(px_of(bytes(data[pos:pos + bpp])))
                    pos += bpp
                got += k
        stream = np.concatenate(out)[:total]
    img = stream.reshape(h, w, 4)
    if not desc & 0x20:
        img = img[::-1]
    if desc & 0x10:
        img = img[:, ::-1]
    img = np.ascontiguousarray(img)
    if heur and not img[..., 3].any():
        img = img.copy()
        img[..., 3] = 255
        meta = dict(meta, alpha=False, channels=meta["channels"] - 1)
    if premul:
        img = img.copy()
        for c in range(3):
            img[..., c] = np.minimum(img[..., c], img[..., 3])
    return 0, img, meta, premul


# ================================================================== generators

def gen_file(rng, **force):
    """Random TGA variant -> (bytes, description)."""
    base = force.get("base", rng.choice([1, 2, 2, 3]))
    rle = force.get("rle", rng.random() < 0.5)
    w = force.get("w", rng.choice([1, 2, 3, 7, 16, 33]))
    h = force.get("h", rng.choice([1, 2, 5, 9]))
    n = w * h
    desc = force.get("origin", rng.randrange(4)) << 4
    cmap = None
    if base == 2:
        depth = force.get("depth", rng.choice([15, 16, 24, 32]))
    elif base == 3:
        depth = force.get("depth", rng.choice([8, 16]))
    else:
        depth = force.get("depth", rng.choice([8, 8, 16]))
    bpp = (depth + 7) // 8
    alpha_bits = force.get("abits", None)
    if alpha_bits is None:
        alpha_bits = {32: 8, 16: rng.choice([0, 1])}.get(depth, 0) if base == 2 else (8 if (base == 3 and depth == 16) else 0)
        if rng.random() < 0.15:
            alpha_bits = rng.choice([0, 1, 8])
    desc |= alpha_bits
    # stored pixel values; low entropy so that runs occur
    palsize = rng.choice([2, 4, 16])
    if base == 1:
        cbits = force.get("cbits", rng.choice([15, 16, 24, 32]))
        esz = (cbits + 7) // 8
        count = force.get("count", rng.choice([1, 5, 16, 200]) if depth == 8 else rng.choice([3, 300, 1000]))
        first = force.get("first", rng.choice([0, 0, 3, 100]) if depth == 8 else rng.choice([0, 7, 65000]))
        cmdata = bytes(rng.getrandbits(8) for _ in range(count * esz))
        cmap = (first, cmdata, cbits, count)
        lo, hi = max(0, first - 2), min((1 << depth) - 1, first + count + 1)
        vals = [rng.randint(lo, hi) for _ in range(palsize)]
        stored = [struct.pack("<B" if depth == 8 else "<H", v) for v in vals]
    else:
        stored = [bytes(rng.getrandbits(8) for _ in range(bpp)) for _ in range(palsize)]
        if force.get("zero_alpha") and depth == 32:
            stored = [s[:3] + b"\0" for s in stored]
    pix = b"".join(rng.choice(stored) for _ in range(n))
    itype = base + (8 if rle else 0)
    if rle:
        stream = rle_encode(pix, bpp, rng, cross=force.get("cross", True), w=w)
        if force.get("overrun"):
            stream += bytes([0x85]) + stored[0]          # a packet past the end: ignored
    else:
        stream = pix
    idfield = bytes(rng.getrandbits(8) for _ in range(force.get("idlen", rng.choice([0, 0, 1, 20]))))
    ext = force.get("ext", rng.choice([None, None, None, 0, 1, 2, 3, 3, 4]))
    if base == 2 and cmap is None and rng.random() < 0.1 and not force.get("nomap"):
        cmap = (0, bytes(3 * 4), 24, 4)                   # colour map of a true-colour image
    data = tga_bytes(w, h, itype, depth, stream, desc=desc, cmap=cmap, idfield=idfield, ext=ext)
    name = f"type {itype} depth {depth} {w}x{h} desc {desc:#x}" + (f" map {cmap[2]}b x{cmap[3]} @{cmap[0]}" if cmap else "") + \
        (f" ext {ext}" if ext is not None else "")
    return data, name


# ================================================================== suites

def decex(data, flags=C.IMGD_STRAIGHT, **kw):
    return it.decex_input(data, flags=flags, hint=C.IMGF_TGA, **kw)


def check_ref(data, straight=True):
    err, img, meta, premul = ref_decode(data)

    def chk(res, out):
        if res != err:
            return f"result {it.err_name(res)}, reference {it.err_name(err)}"
        if err:
            return None
        d = it.parse_decex(out)
        exp = img
        if straight:
            if premul:
                exp = it.unpremul(img)
        else:
            exp = img if premul else it.premul(img)
        if d["px"] is None or d["px"].shape != exp.shape:
            return f"size {None if d['px'] is None else d['px'].shape} != {exp.shape}"
        if not np.array_equal(d["px"], exp):
            m = it.max_abs(d["px"], exp)
            y, x = np.argwhere(np.abs(d["px"].astype(int) - exp.astype(int)).max(-1) > 0)[0]
            return f"pixels differ (max {m}) at ({x},{y}): got {list(d['px'][y, x])} exp {list(exp[y, x])}"
        mm = d["meta"]
        got = dict(w=mm["w"], h=mm["h"], bits=mm["bits"], model=mm["model"], channels=mm["channels"],
                   alpha=bool(mm["flags"] & C.IMF_ALPHA), premul=bool(mm["flags"] & C.IMF_SRC_PREMUL))
        if got != meta:
            return f"meta {got} != {meta}"
        if not mm["flags"] & C.IMF_LOSSLESS:
            return "IMF_LOSSLESS missing"
        return None
    return chk


def suite_decode(rng, n):
    recs = []
    for i in range(n):
        data, name = gen_file(rng)
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"#{i} {name}"))
        if i % 4 == 0:
            recs.append(Rec(it.OP_DECEX, 0, decex(data, flags=0), check_ref(data, False), f"#{i} {name} PARGB"))
    # systematic: every type x depth x origin x RLE crossing / not crossing
    for base, depths in ((1, (8, 16)), (2, (15, 16, 24, 32)), (3, (8, 16))):
        for depth in depths:
            for origin in range(4):
                for rle in (False, True):
                    for cross in (True, False):
                        if not rle and not cross:
                            continue
                        data, name = gen_file(rng, base=base, depth=depth, origin=origin, rle=rle, cross=cross,
                                              w=rng.choice([5, 9]), h=4)
                        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"sys {name} cross={cross}"))
    # colour maps: entry sizes, first index, indices outside the map
    for cbits in (15, 16, 24, 32):
        for depth, first, count in ((8, 0, 256), (8, 250, 10), (8, 3, 1), (16, 0, 300), (16, 65530, 10)):
            for abits in (0, 1, 8):
                data, name = gen_file(rng, base=1, depth=depth, cbits=cbits, first=first, count=count, abits=abits)
                recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"map {name} abits {abits}"))
    # alpha rules
    for ext in (None, 0, 1, 2, 3, 4, 5, 255):
        for depth, abits in ((32, 8), (32, 0), (16, 1), (16, 0), (24, 0), (15, 0)):
            data, name = gen_file(rng, base=2, depth=depth, abits=abits, ext=ext)
            recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"alpha {name}"))
            recs.append(Rec(it.OP_DECEX, 0, decex(data, flags=0), check_ref(data, False), f"alpha {name} PARGB"))
        data, name = gen_file(rng, base=3, depth=16, ext=ext)
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"alpha {name}"))
        for cbits in (16, 32):
            data, name = gen_file(rng, base=1, depth=8, cbits=cbits, abits=rng.choice([0, 1]), ext=ext)
            recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"alpha {name}"))
    for ext in (None, 3):
        data, name = gen_file(rng, base=2, depth=32, zero_alpha=True, ext=ext)
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"all-zero alpha {name}"))
    # footer / extension variants
    base, _ = gen_file(rng, base=2, depth=32, ext=None)
    for nm, foot in (("footer ext 0", struct.pack("<II", 0, 0) + SIG),
                     ("footer ext beyond", struct.pack("<II", 1 << 30, 0) + SIG),
                     ("footer ext 5", struct.pack("<II", 5, 0) + SIG),
                     ("bad signature", struct.pack("<II", 18, 0) + b"TRUEVISION-XFILE,\0")):
        data = base + foot
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), nm))
    area = bytearray(495)
    area[494] = 4
    for size in (494, 495, 600):
        struct.pack_into("<H", area, 0, size)
        data = base + bytes(area) + struct.pack("<II", len(base), 0) + SIG
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"extension size {size}"))
    # RLE overrun, ID fields, x / y origin fields ignored
    for i in range(10):
        data, name = gen_file(rng, rle=True, overrun=True)
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"overrun {name}"))
    data, name = gen_file(rng, idlen=255)
    recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"id 255 {name}"))
    return recs


def suite_errors(rng):
    recs = []
    good, _ = gen_file(rng, base=2, depth=32, rle=True, w=16, h=8, ext=3)
    for n in (0, 1, 5, 17, 18, 19, 30, len(good) // 2, len(good) - 27):
        data = good[:n]
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data) if n >= 2 else it.expect_err(C.ERR_TRUNC), f"cut {n}"))
    raw, _ = gen_file(rng, base=2, depth=24, rle=False, w=16, h=8, ext=None)
    for n in (18, 19, len(raw) - 1):
        recs.append(Rec(it.OP_DECEX, 0, decex(raw[:n]), check_ref(raw[:n]), f"raw cut {n}"))

    def mod(data, off, val, fmt="<B"):
        b = bytearray(data)
        struct.pack_into(fmt, b, off, val)
        return bytes(b)
    cases = {
        "type 0": mod(raw, 2, 0), "type 32": mod(raw, 2, 32), "type 33": mod(raw, 2, 33), "type 4": mod(raw, 2, 4),
        "type 8": mod(raw, 2, 8), "type 12": mod(raw, 2, 12), "type 130": mod(raw, 2, 130),
        "cmap type 2": mod(raw, 1, 2), "depth 12": mod(raw, 16, 12), "depth 48": mod(raw, 16, 48),
        "interleave 1": mod(raw, 17, 0x40), "interleave 2": mod(raw, 17, 0x80), "w 0": mod(raw, 12, 0, "<H"),
        "h 0": mod(raw, 14, 0, "<H"), "mapped without map": mod(raw, 2, 1),
        "mapped depth 24": tga_bytes(2, 2, 1, 24, bytes(12), cmap=(0, bytes(6), 24, 2)),
        "map entry 8": tga_bytes(2, 2, 1, 8, bytes(4), cmap=(0, bytes(2), 8, 2)),
        "map length 0": tga_bytes(2, 2, 1, 8, bytes(4), cmap=(0, b"", 24, 0)),
        "map truncated": tga_bytes(2, 2, 1, 8, b"", cmap=(0, bytes(3 * 40), 24, 300))[:200],
        "grey 24": mod(mod(raw, 2, 3), 16, 24), "grey 32": mod(mod(raw, 2, 3), 16, 32),
    }
    for nm, data in cases.items():
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), nm))
    big = tga_bytes(65535, 65535, 10, 32, b"\xff" + bytes(4))
    recs.append(Rec(it.OP_DECEX, 0, decex(big), it.expect_err(C.ERR_LIMIT), "65535^2"))
    # huge declared sizes with tiny data fail before the surface is allocated
    for itype in (2, 10):
        data = tga_bytes(16384, 16384, itype, 32, bytes([0xFF, 0, 0, 0, 0]))
        recs.append(Rec(it.OP_DECEX, 0, decex(data), it.expect_err(C.ERR_TRUNC), f"16384^2 type {itype}, 5 bytes"))
    recs.append(Rec(it.OP_DECEX, 0, decex(raw, maxpx=16 * 8 - 1), it.expect_err(C.ERR_LIMIT), "maxpx - 1"))
    recs.append(Rec(it.OP_DECEX, 0, decex(raw, maxpx=16 * 8), check_ref(raw), "maxpx exact"))
    recs.append(Rec(it.OP_DECEX, 0, decex(raw, cancel=1), it.expect_err(C.ERR_CANCEL), "cancel"))
    recs.append(Rec(it.OP_DECEX, 0, decex(raw, frame=1), it.expect_err(C.ERR_UNSUPPORTED), "frame 1"))
    # probe flag: meta only
    def chk_probe(res, out):
        d = it.parse_decex(out)
        return None if res == 0 and d["px"] is None and (d["meta"]["w"], d["meta"]["h"]) == (16, 8) else f"{res} {d['meta']}"
    recs.append(Rec(it.OP_DECEX, 0, decex(raw, flags=C.IMGD_PROBE), chk_probe, "IMGD_PROBE"))
    return recs


def suite_detect(rng):
    recs = []
    for i in range(60):
        data, name = gen_file(rng)
        err, img, meta, _ = ref_decode(data)
        if err:
            continue

        def chk(res, out, data=data):
            # the strict probe may refuse unusual variants; if it accepts, the decode must work
            if res == 0:
                return None if it.parse_decex(out)["px"] is not None else "no pixels"
            return None if res == C.ERR_UNSUPPORTED else f"result {it.err_name(res)}"
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, C.IMGD_STRAIGHT), chk, f"no hint {name}"))
        recs.append(Rec(it.OP_DECEX, 0, decex(data), check_ref(data), f"hint {name}"))
    return recs


# ------------------------------------------------------------------ ffmpeg

def run_ffmpeg(suite, runner, rng):
    if not it.have_exe("ffmpeg"):
        suite.notes.append("ffmpeg not found -- skipped")
        return
    work = Path(tempfile.mkdtemp(prefix="tgaff_", dir=runner.work))
    recs, notes = [], []
    # a smooth + noisy source image with alpha
    for w, h in ((37, 23), (128, 3), (1, 1), (300, 200)):
        src = it.rand_bgra(rng, w, h, alpha="random")
        yy, xx = np.mgrid[0:h, 0:w]
        src[..., 0] = np.where((xx // 5) % 2 == 0, src[..., 0], (xx * 7) % 256)
        src[..., 3] = np.where(src[..., 3] == 0, 1, src[..., 3])
        png = work / f"s{w}x{h}.png"
        png.write_bytes(it.png_make(src[..., [2, 1, 0, 3]]))
        for pf in ("bgra", "bgr24", "rgb555le", "gray", "pal8"):
            for rle in (0, 1):
                out = work / f"o_{w}x{h}_{pf}_{rle}.tga"
                p = subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", str(png), "-c:v", "targa", "-rle", str(rle),
                                       "-pix_fmt", pf, str(out)], capture_output=True, timeout=120)
                if p.returncode != 0 or not out.exists():
                    notes.append(f"ffmpeg cannot write {pf} rle {rle}")
                    continue
                ref = it.ffmpeg_decode(out)
                data = out.read_bytes()
                if ref is None:
                    notes.append(f"ffmpeg cannot read back {pf}")
                    continue

                def chk(res, o, ref=ref, pf=pf, data=data):
                    if res != 0:
                        return f"error {it.err_name(res)}"
                    px = it.parse_decex(o)["px"]
                    if not np.array_equal(px, ref):
                        return f"differs from ffmpeg (max {it.max_abs(px, ref)})"
                    e = check_ref(data)(res, o)
                    return e
                recs.append(Rec(it.OP_DECEX, 0, decex(data), chk, f"ffmpeg-written {pf} rle {rle} {w}x{h}"))
    # Python-written variants ffmpeg understands the same way (no all-zero alpha, no 16-bit
    # alpha bits, no out-of-map indices, no extension area alpha types, 8-bit indices)
    files = []
    for i in range(80):
        base = rng.choice([1, 2, 2, 3])
        kw = dict(base=base, ext=None, nomap=True)
        if base == 1:
            kw.update(depth=8, cbits=rng.choice([24, 32]), first=0, count=256)
        elif base == 2:
            kw.update(depth=rng.choice([15, 16, 24, 32]), abits=0)
            if kw["depth"] == 32:
                kw["abits"] = 8
        else:
            kw.update(depth=8)
        data, name = gen_file(rng, **kw)
        err, img, meta, _ = ref_decode(data)
        if err or (img[..., 3] == 0).all():
            continue
        if data[16] == 15 and data[17] & 0x10:
            continue            # ffmpeg bug: its right-to-left flip has no case for 15 bit
        p = work / f"py{i}.tga"
        p.write_bytes(data)
        files.append((p, data, name))
    for p, data, name in files:
        ref = it.ffmpeg_decode(p)
        if ref is None:
            notes.append(f"ffmpeg cannot read {name}")
            continue

        def chk(res, o, ref=ref, data=data):
            if res != 0:
                return f"error {it.err_name(res)}"
            px = it.parse_decex(o)["px"]
            if not np.array_equal(px, ref):
                return f"differs from ffmpeg (max {it.max_abs(px, ref)})"
            return None
        recs.append(Rec(it.OP_DECEX, 0, decex(data), chk, f"ffmpeg reads {name}"))
    it.run_suite(runner, suite, recs)
    suite.notes += sorted(set(notes))


# ------------------------------------------------------------------ encoder

def imgopt(bitdepth=0, flags=0):
    return struct.pack("<12IQQ", C.IMGF_TGA, 0, flags, bitdepth, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)


def parse_tga_structure(data, w, h, bpp, rle):
    """Checks header, packets per row, extension area and footer -> None / message."""
    if data[2] != (10 if rle else 2) or struct.unpack_from("<HH", data, 12) != (w, h) or data[16] != bpp * 8:
        return f"header {data[:18].hex()}"
    if data[17] != (8 if bpp == 4 else 0) or data[0] or data[1]:
        return f"descriptor / id / map {data[17]:#x}"
    if data[-18:] != SIG:
        return "no v2 signature"
    ext = struct.unpack_from("<I", data, len(data) - 26)[0]
    if ext + 495 + 26 != len(data) or struct.unpack_from("<H", data, ext)[0] != 495:
        return f"extension offset {ext} / size"
    if data[ext + 494] != (3 if bpp == 4 else 0):
        return f"attributes type {data[ext + 494]}"
    pos = 18
    for y in range(h):
        x = 0
        while x < w:
            b = data[pos]
            pos += 1
            k = (b & 0x7F) + 1 if rle else w
            if rle:
                pos += bpp if b & 0x80 else k * bpp
            else:
                pos += w * bpp - 1
            x += k
        if x != w:
            return f"row {y}: packets cross the row ({x} != {w})"
    if pos != ext:
        return f"pixel data ends at {pos}, extension at {ext}"
    bound = 18 + h * (w * bpp + w // 128 + 2) + 495 + 26
    if len(data) > bound:
        return f"size {len(data)} above the bound {bound}"
    return None


def suite_encode(rng, runner):
    recs = []
    work = Path(tempfile.mkdtemp(prefix="tgaenc_", dir=runner.work))
    ff = it.have_exe("ffmpeg")
    encoded = []
    cases = []
    for w, h in ((1, 1), (5, 3), (129, 2), (300, 7), (64, 64)):
        for kind in ("noise", "runs", "flat"):
            px = it.rand_bgra(rng, w, h, alpha="mixed")
            if kind == "runs":
                px = np.repeat(px[:, ::max(1, w // 7)], max(1, w // 7) + 1, 1)[:, :w].copy()
            elif kind == "flat":
                px[:] = px[0, 0]
            cases.append((w, h, np.ascontiguousarray(px), kind))
    for w, h, px, kind in cases:
        for sfmt in (C.FMT_ARGB, C.FMT_PARGB, C.FMT_XRGB, C.FMT_A8):
            if sfmt == C.FMT_A8:
                src = px[..., 3].copy()
                straight = np.stack([src, src, src, np.full_like(src, 255)], -1)
                sin = it.surface_input(src, sfmt)
            elif sfmt == C.FMT_PARGB:
                pm = it.premul(px)
                straight = it.unpremul(pm)
                sin = it.surface_input(pm, sfmt)
            elif sfmt == C.FMT_XRGB:
                straight = px.copy()
                straight[..., 3] = 255
                sin = it.surface_input(px, sfmt)
            else:
                straight = px
                sin = it.surface_input(px, sfmt)
            for bits in (0, 32, 24):
                for rle in (False, True):
                    bpp = 3 if bits == 24 else 4
                    if bpp == 4:
                        exp = straight
                    else:
                        pm = it.premul(straight).astype(np.int64)
                        exp = np.concatenate([np.minimum(255, pm[..., :3] + 255 - pm[..., 3:4]),
                                              np.full(pm.shape[:2] + (1,), 255)], -1).astype(np.uint8)
                    name = f"encode {w}x{h} {kind} sfmt {sfmt} bits {bits} rle {int(rle)}"
                    opt = imgopt(bits, C.IOF_RLE if rle else 0)

                    def chk(res, out, exp=exp, w=w, h=h, bpp=bpp, rle=rle, name=name):
                        if res != 0:
                            return f"error {it.err_name(res)}"
                        data = bytes(out)
                        e = parse_tga_structure(data, w, h, bpp, rle)
                        if e:
                            return e
                        err, img, meta, _ = ref_decode(data)
                        if err or not np.array_equal(img, exp):
                            return f"reference decode {it.err_name(err)} differs (max {it.max_abs(img, exp) if img is not None else '-'})"
                        encoded.append((data, exp, name))
                        return None
                    recs.append(Rec(it.OP_ENCODE, 0, opt + sin, chk, name))
    s = it.Suite("encode", runner.target)
    it.run_suite(runner, s, recs)
    # our decoder and ffmpeg on the encoded files
    recs2 = []
    for i, (data, exp, name) in enumerate(encoded):
        recs2.append(Rec(it.OP_DECEX, 0, decex(data), it.expect_decex(exp), f"decode {name}"))
    it.run_suite(runner, s, recs2)
    if ff:
        bad = 0
        for i, (data, exp, name) in enumerate(encoded[::3]):
            p = work / f"e{i}.tga"
            p.write_bytes(data)
            ref = it.ffmpeg_decode(p)
            if ref is None or not np.array_equal(ref, exp):
                bad += 1
                s.fail(f"ffmpeg decode of {name}: {'fails' if ref is None else 'max ' + str(it.max_abs(ref, exp))}")
            else:
                s.ok()
        s.notes.append(f"ffmpeg read {len(encoded[::3]) - bad} / {len(encoded[::3])} encoded files identically")
    # errors
    recs3 = [Rec(it.OP_ENCODE, 0, imgopt(8) + it.surface_input(cases[0][2], C.FMT_ARGB), it.expect_err(C.ERR_UNSUPPORTED), "bits 8"),
             Rec(it.OP_ENCODE, 0, imgopt(16) + it.surface_input(cases[0][2], C.FMT_ARGB), it.expect_err(C.ERR_UNSUPPORTED), "bits 16")]
    it.run_suite(runner, s, recs3)
    return s


def suite_fuzz(rng, n):
    recs = []
    corpus = [gen_file(rng)[0] for _ in range(60)]
    corpus += [gen_file(rng, rle=True, base=b)[0] for b in (1, 2, 3) for _ in range(10)]
    for i in range(n):
        m = it.mutate(rng, rng.choice(corpus), keep=rng.choice([0, 0, 18]))
        flags = rng.choice([0, C.IMGD_STRAIGHT, C.IMGD_PROBE])
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(m, flags, hint=rng.choice([0, C.IMGF_TGA, C.IMGF_TGA])),
                        it.any_result, f"fuzz #{i}"))
        if i % 5 == 0:
            recs.append(Rec(it.OP_PROBE, rng.choice([0, C.IMGF_TGA]), m, it.any_result, f"fuzz probe #{i}"))
    return recs


# ================================================================== main

def main():
    ap = it.argparser(__doc__, SUITES)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        it.build_codectest(targets, out)
    suites = []
    for target in targets:
        print(f"==== {target}")
        runner = it.Runner(target, out, work_name="tga")

        def run(name, fn):
            s = it.Suite(name, target)
            it.run_suite(runner, s, fn())
            suites.append(s)
        if want("decode"):
            run("decode", lambda: suite_decode(random.Random(a.seed), 600))
        if want("ffmpeg") and not a.no_ext:
            s = it.Suite("ffmpeg", target)
            run_ffmpeg(s, runner, random.Random(a.seed + 1))
            suites.append(s)
        if want("encode"):
            suites.append(suite_encode(random.Random(a.seed + 2), runner))
        if want("detect"):
            run("detect", lambda: suite_detect(random.Random(a.seed + 3)))
        if want("errors"):
            run("errors", lambda: suite_errors(random.Random(a.seed + 4)))
        if want("fuzz"):
            run("fuzz", lambda: suite_fuzz(random.Random(a.seed + 5), a.fuzz))
    ok = it.report(suites, "tga_check")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
