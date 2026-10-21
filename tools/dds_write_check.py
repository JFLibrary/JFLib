#!/usr/bin/env python3
"""Checks of the DDS writer (dds_encode in ASMLIB/src/img/dds.s, CB) through apps/codectest
(op 252 img_encode, op 240 img_decode_ex, op 106 raw IMGMETA, ops 111 / 112 in dds_ops.s).

Suites
  header     every IO_DDSFMT (1..21) x linear / sRGB x mips x cube: the header fields against
             DirectXTex's rules (legacy FourCC / mask header when one exists, else DX10;
             flags CAPS | HEIGHT | WIDTH | PIXELFORMAT | MIPMAPCOUNT | LINEARSIZE / PITCH,
             pitch / linear size, depth 1, mip count, caps / caps2, DX10 fields) and the file
             length (header + faces x the level sizes)
  raw        uncompressed formats: the stored pixel bytes equal a Python model of the
             conversion (exact) for ARGB / PARGB / XRGB / A8 sources, and our reader decodes
             them to the expected pixels; IMGMETA (IM_DXGI, frames, sRGB flag)
  bc         BC formats: the stored blocks decode (Python reference) to what our reader
             returns for every mip level; PSNR against the source
  mips       level sizes down to 1 x 1 (odd / thin sizes); Fant / box levels against a
             Python area average of the previous level (premultiplied, +-2), the other filters
             close to it; IOF_SRGB filters in linear light (checker mean 188, not 128)
  cube       4 x 3 cross -> cube map (header, six faces with their mips) -> our reader gives
             the cross back; wrong cross sizes -> ERR_DIMS
  wic        WIC decodes our BC1 / BC2 / BC3 (legacy and DX10 incl. sRGB) and B8G8R8A8 /
             B8G8R8X8 files identically to our reader (Windows only)
  ffmpeg     ffmpeg decodes our DXT1 / DXT3 / DXT5 / BC4U / BC4S / BC5U / BC5S, A8R8G8B8,
             X8R8G8B8, A8B8G8R8, R5G6B5, A1R5G5B5, A4R4G4B4, L8, A8, R8G8B8 files like our
             reader (BC +-2, 5 / 6-bit replication +-4, else exact)
  errors     unknown IO_DDSFMT, cancel (IO_CANCEL set), empty surfaces
  big        synthetic images (op 111): 16384^2 BC1 + mips, 8192^2 B8G8R8A8 + mips, 4097 x 1
             and 1 x 4097 BC7 + mips (13 levels), 1 x 1 in every format; lengths / hashes
  perf       DDS encode (header, mips, pool) of 4096^2 BC1 / BC3 / BC7 (op 111)

  python tools/dds_write_check.py --target both --out build/cb
"""

import random
import struct
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
import dds_check as ref  # noqa: E402

C = it.C
Rec = it.Rec
SUITES = "header,raw,bc,mips,cube,wic,ffmpeg,errors,big,perf"
OP_RAWMETA = 106
OP_ENC_SYN, OP_ENC_T = 111, 112


def fourcc(s):
    return struct.unpack("<I", s.encode())[0]


# DirectXTex's legacy pixel formats: flags, FourCC, bit count, R, G, B, A masks
PF = {"DXT1": (4, fourcc("DXT1"), 0, 0, 0, 0, 0), "DXT3": (4, fourcc("DXT3"), 0, 0, 0, 0, 0),
      "DXT5": (4, fourcc("DXT5"), 0, 0, 0, 0, 0), "BC4U": (4, fourcc("BC4U"), 0, 0, 0, 0, 0),
      "BC4S": (4, fourcc("BC4S"), 0, 0, 0, 0, 0), "BC5U": (4, fourcc("BC5U"), 0, 0, 0, 0, 0),
      "BC5S": (4, fourcc("BC5S"), 0, 0, 0, 0, 0),
      "A8R8G8B8": (0x41, 0, 32, 0xFF0000, 0xFF00, 0xFF, 0xFF000000),
      "X8R8G8B8": (0x40, 0, 32, 0xFF0000, 0xFF00, 0xFF, 0),
      "A8B8G8R8": (0x41, 0, 32, 0xFF, 0xFF00, 0xFF0000, 0xFF000000),
      "A1R5G5B5": (0x41, 0, 16, 0x7C00, 0x3E0, 0x1F, 0x8000),
      "A4R4G4B4": (0x41, 0, 16, 0xF00, 0xF0, 0xF, 0xF000),
      "R5G6B5": (0x40, 0, 16, 0xF800, 0x7E0, 0x1F, 0),
      "L8": (0x20000, 0, 8, 0xFF, 0, 0, 0), "A8": (0x2, 0, 8, 0, 0, 0, 0xFF),
      "R8G8B8": (0x40, 0, 24, 0xFF0000, 0xFF00, 0xFF, 0)}

# IO_DDSFMT -> (name, kind, bcfmt / conversion, bytes per block / pixel, legacy pf, DXGI, sRGB DXGI)
WF = {1: ("BC1", "bc", 1, 8, "DXT1", 71, 72), 2: ("BC2", "bc", 2, 16, "DXT3", 74, 75),
      3: ("BC3", "bc", 3, 16, "DXT5", 77, 78), 4: ("BC4", "bc", 4, 8, "BC4U", 80, 0),
      5: ("BC5", "bc", 5, 16, "BC5U", 83, 0), 6: ("BC6H", "bc", 6, 16, None, 95, 0),
      7: ("BC7", "bc", 7, 16, None, 98, 99), 8: ("BGRA8", "raw", "bgra", 4, "A8R8G8B8", 87, 91),
      9: ("BGRX8", "raw", "bgrx", 4, "X8R8G8B8", 88, 93), 10: ("RGBA8", "raw", "rgba", 4, "A8B8G8R8", 28, 29),
      11: ("BGR5A1", "raw", "555", 2, "A1R5G5B5", 86, 0), 12: ("BGRA4", "raw", "4444", 2, "A4R4G4B4", 115, 0),
      13: ("B5G6R5", "raw", "565", 2, "R5G6B5", 85, 0), 14: ("L8", "raw", "lum", 1, "L8", 61, 0),
      15: ("A8", "raw", "a8", 1, "A8", 65, 0), 16: ("R8G8", "raw", "rg", 2, None, 49, 0),
      17: ("BC4S", "bc", 8, 8, "BC4S", 81, 0), 18: ("BC5S", "bc", 9, 16, "BC5S", 84, 0),
      19: ("BC6HS", "bc", 10, 16, None, 96, 0), 20: ("R8", "raw", "lum", 1, None, 61, 0),
      21: ("BGR8", "raw", "bgr", 3, "R8G8B8", 0, 0)}


def opt(ddsfmt, flags=0, effort=0, mipf=0, athresh=0):
    o = bytearray(64)
    struct.pack_into("<I", o, 0, C.IMGF_DDS)
    struct.pack_into("<I", o, 8, flags)
    struct.pack_into("<I", o, 24, athresh)
    struct.pack_into("<I", o, 32, effort)
    struct.pack_into("<I", o, 36, ddsfmt)
    struct.pack_into("<I", o, 40, mipf)
    return bytes(o)


def enc_rec(img, ddsfmt, flags=0, fmt=None, chk=None, name="", **kw):
    return Rec(it.OP_ENCODE, 0, opt(ddsfmt, flags, **kw) + it.surface_input(img, C.FMT_ARGB if fmt is None else fmt),
               chk, name)


def level_dims(w, h, levels):
    return [(max(1, w >> l), max(1, h >> l)) for l in range(levels)]


def level_bytes(ddsfmt, w, h):
    _, kind, _, bpx, _, _, _ = WF[ddsfmt]
    if kind == "bc":
        return ((w + 3) // 4) * ((h + 3) // 4) * bpx
    return w * h * bpx


def expected_header(ddsfmt, w, h, levels, cube, srgb):
    name, kind, _, bpx, pf, dxgi, dxgis = WF[ddsfmt]
    use_srgb = srgb and dxgis != 0
    dx10 = use_srgb or pf is None
    flags = 0x1007 | 0x20000 | (0x80000 if kind == "bc" else 0x8)
    pitch = level_bytes(ddsfmt, w, h) if kind == "bc" else w * bpx
    caps = 0x1000 | (0x400008 if levels > 1 else 0) | (0x8 if cube else 0)
    caps2 = 0xFE00 if cube else 0
    hdr = dict(size=124, flags=flags, h=h, w=w, pitch=pitch, depth=1, mips=levels, pfsize=32,
               pf=(4, fourcc("DX10"), 0, 0, 0, 0, 0) if dx10 else PF[pf], caps=caps, caps2=caps2)
    if dx10:
        hdr["dx10"] = (dxgis if use_srgb else dxgi, 3, 4 if cube else 0, 1, 0)
    return hdr


def parse_header(b):
    if len(b) < 128 or b[:4] != b"DDS ":
        return None
    v = struct.unpack_from("<7I", b, 4)
    d = dict(size=v[0], flags=v[1], h=v[2], w=v[3], pitch=v[4], depth=v[5], mips=v[6],
             pfsize=struct.unpack_from("<I", b, 76)[0], pf=struct.unpack_from("<7I", b, 80),
             caps=struct.unpack_from("<I", b, 108)[0], caps2=struct.unpack_from("<I", b, 112)[0],
             reserved=b[32:76] + b[116:128])
    if d["pf"][1] == fourcc("DX10"):
        d["dx10"] = struct.unpack_from("<5I", b, 128)
    return d


# ================================================================== conversion model

def lum(bgra):
    b, g, r = (bgra[..., i].astype(np.int64) for i in range(3))
    return ((7471 * b + 38470 * g + 19595 * r) >> 16).astype(np.uint8)


def q(c, mx):
    return ((c.astype(np.int64) * mx + 127) // 255).astype(np.int64)


def convert_raw(conv, bgra):
    """straight BGRA (h, w, 4) -> the file's pixel bytes (packed rows)."""
    b, g, r, a = (bgra[..., i] for i in range(4))
    if conv == "bgra":
        return bgra.tobytes()
    if conv == "bgrx":
        x = bgra.copy()
        x[..., 3] = 255
        return x.tobytes()
    if conv == "rgba":
        return bgra[..., [2, 1, 0, 3]].tobytes()
    if conv == "555":
        v = q(b, 31) | q(g, 31) << 5 | q(r, 31) << 10 | (a.astype(np.int64) >= 128) << 15
        return v.astype("<u2").tobytes()
    if conv == "4444":
        v = q(b, 15) | q(g, 15) << 4 | q(r, 15) << 8 | q(a, 15) << 12
        return v.astype("<u2").tobytes()
    if conv == "565":
        v = q(b, 31) | q(g, 63) << 5 | q(r, 31) << 11
        return v.astype("<u2").tobytes()
    if conv == "lum":
        return lum(bgra).tobytes()
    if conv == "a8":
        return a.tobytes()
    if conv == "rg":
        return np.stack([r, g], -1).tobytes()
    if conv == "bgr":
        return bgra[..., :3].tobytes()
    raise ValueError(conv)


def expand(v, bits):
    m = (1 << bits) - 1
    return ((v.astype(np.int64) * 510 + m) // (2 * m)).astype(np.uint8)


def decoded_raw(conv, bgra):
    """What our reader returns for the converted pixels (straight BGRA)."""
    b, g, r, a = (bgra[..., i] for i in range(4))
    one = np.full_like(a, 255)
    if conv == "bgra":
        return bgra.copy()
    if conv in ("bgrx", "bgr"):
        return np.stack([b, g, r, one], -1)
    if conv == "rgba":
        return bgra.copy()
    if conv == "555":
        return np.stack([expand(q(b, 31), 5), expand(q(g, 31), 5), expand(q(r, 31), 5),
                         np.where(a >= 128, 255, 0).astype(np.uint8)], -1)
    if conv == "4444":
        return np.stack([expand(q(b, 15), 4), expand(q(g, 15), 4), expand(q(r, 15), 4), expand(q(a, 15), 4)], -1)
    if conv == "565":
        return np.stack([expand(q(b, 31), 5), expand(q(g, 63), 6), expand(q(r, 31), 5), one], -1)
    if conv == "lum":
        y = lum(bgra)
        return np.stack([y, y, y, one], -1)
    if conv == "a8":
        z = np.zeros_like(a)
        return np.stack([z, z, z, a], -1)
    if conv == "rg":
        return np.stack([np.zeros_like(a), g, r, one], -1)
    raise ValueError(conv)


def decex(data, frame=0):
    return it.decex_input(data, C.IMGD_STRAIGHT, frame=frame)


def rand_img(nr, w, h, alpha=True):
    a = nr.integers(0, 256, (h, w, 4), dtype=np.uint8)
    if not alpha:
        a[..., 3] = 255
    return a


# ================================================================== suites

def suite_header(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    recs = []
    for ddsfmt in WF:
        for srgb in (False, True):
            for mips in (False, True):
                for cube in (False, True):
                    if cube and ddsfmt not in (1, 7, 8, 14, 21):
                        continue
                    w, h = (40, 30) if cube else (13, 7)
                    img = rand_img(nr, w, h)
                    flags = (C.IOF_SRGB if srgb else 0) | (C.IOF_MIPMAPS if mips else 0) | (C.IOF_CUBE_CROSS if cube else 0)
                    fw, fh = (w // 4, h // 3) if cube else (w, h)
                    levels = max(fw, fh).bit_length() if mips else 1
                    exp = expected_header(ddsfmt, fw, fh, levels, cube, srgb)
                    nfaces = 6 if cube else 1
                    exp_len = (148 if "dx10" in exp else 128) + nfaces * sum(level_bytes(ddsfmt, lw, lh)
                                                                            for lw, lh in level_dims(fw, fh, levels))

                    def chk(res, out, exp=exp, exp_len=exp_len):
                        if res != 0:
                            return f"error {it.err_name(res)}"
                        d = parse_header(out)
                        if d is None:
                            return "no DDS header"
                        for k, v in exp.items():
                            if d.get(k) != v:
                                return f"{k} = {d.get(k)}, expected {v}"
                        if "dx10" not in exp and "dx10" in d:
                            return "unexpected DX10 header"
                        if any(d["reserved"]):
                            return "reserved fields not zero"
                        if len(out) != exp_len:
                            return f"length {len(out)} != {exp_len}"
                        return None
                    recs.append(enc_rec(img, ddsfmt, flags, chk=chk,
                                        name=f"{WF[ddsfmt][0]} srgb={srgb} mips={mips} cube={cube}"))
    it.run_suite(runner, suite, recs)


def suite_raw(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    recs = []
    for ddsfmt, (name, kind, conv, bpx, pf, dxgi, dxgis) in WF.items():
        if kind != "raw":
            continue
        for w, h in ((1, 1), (5, 3), (16, 9), (33, 2)):
            img = rand_img(nr, w, h)
            img[0, 0] = (1, 2, 3, 0)
            if w > 1:
                img[0, 1] = (250, 128, 7, 255)
            for srcfmt in ("argb", "pargb", "xrgb", "a8"):
                if srcfmt == "argb":
                    src, straight = img, img
                elif srcfmt == "pargb":
                    src = it.premul(img)
                    straight = it.unpremul(src)
                elif srcfmt == "xrgb":
                    src = img.copy()
                    src[..., 3] = 17
                    straight = img.copy()
                    straight[..., 3] = 255
                else:
                    if w * h > 64:
                        continue
                    src = img[..., 0].copy()
                    straight = np.stack([src, src, src, np.full_like(src, 255)], -1)
                sf = {"argb": C.FMT_ARGB, "pargb": C.FMT_PARGB, "xrgb": C.FMT_XRGB, "a8": C.FMT_A8}[srcfmt]
                data = convert_raw(conv, straight)
                decoded = decoded_raw(conv, straight)

                def chk(res, out, data=data):
                    if res != 0:
                        return f"error {it.err_name(res)}"
                    hl = 148 if out[84:88] == b"DX10" else 128
                    if out[hl:] != data:
                        return f"pixel bytes differ ({len(out) - hl} vs {len(data)})"
                    return None
                recs.append(Rec(it.OP_ENCODE, 0, opt(ddsfmt) + it.surface_input(src, sf), chk, f"{name} {w}x{h} {srcfmt}"))
                if srcfmt == "argb":
                    # our reader: two records, file produced on the fly below
                    recs.append(("dec", ddsfmt, img, decoded, name, w, h))
    # run encodes, then decode the argb files
    enc = [r for r in recs if isinstance(r, Rec)]
    results = it.run_suite(runner, suite, enc)
    files = {}
    k = 0
    for r in recs:
        if isinstance(r, Rec):
            files[r.name] = results[k]
            k += 1
    dec_recs = []
    for r in recs:
        if isinstance(r, tuple):
            _, ddsfmt, img, decoded, name, w, h = r
            res = files.get(f"{name} {w}x{h} argb")
            if not res or res[0] != 0:
                continue
            dec_recs.append(Rec(it.OP_DECEX, 0, decex(res[1]), it.expect_decex(decoded), f"read {name} {w}x{h}"))
            dxgi = WF[ddsfmt][5]

            def mchk(res_, out, dxgi=dxgi, name=name):
                if res_ != 0:
                    return f"error {it.err_name(res_)}"
                m = ref.raw_meta(out)
                if name in ("BGRA8", "BGRX8", "RGBA8") and m["dxgi"] != dxgi:
                    return f"IM_DXGI {m['dxgi']} != {dxgi}"
                return None if m["frames"] == 1 else f"frames {m['frames']}"
            dec_recs.append(Rec(OP_RAWMETA, 0, res[1], mchk, f"meta {name} {w}x{h}"))
    it.run_suite(runner, suite, dec_recs)
    # sRGB variants carry IMF_SRGB
    img = rand_img(nr, 8, 8)
    recs = []
    for ddsfmt in (1, 2, 3, 7, 8, 9, 10):
        res = runner.one(it.OP_ENCODE, 0, opt(ddsfmt, C.IOF_SRGB) + it.surface_input(img, C.FMT_ARGB))

        def schk(r, out, d=ddsfmt):
            m = ref.raw_meta(out)
            return None if r == 0 and m["flags"] & C.IMF_SRGB and m["dxgi"] == WF[d][6] else f"{it.err_name(r)} {m}"
        recs.append(Rec(OP_RAWMETA, 0, res[1], schk, f"sRGB meta {WF[ddsfmt][0]}"))
    it.run_suite(runner, suite, recs)


def bc_frames(fileb, ddsfmt, w, h, levels, faces=1):
    """-> list of (level, face, block bytes) from a written file."""
    hl = 148 if fileb[84:88] == b"DX10" else 128
    pos = hl
    out = []
    for f in range(faces):
        for l, (lw, lh) in enumerate(level_dims(w, h, levels)):
            n = level_bytes(ddsfmt, lw, lh)
            out.append((l, f, lw, lh, fileb[pos:pos + n]))
            pos += n
    return out, pos


def suite_bc(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    recs, jobs = [], []
    for ddsfmt, (name, kind, bcf, bpx, pf, dxgi, dxgis) in WF.items():
        if kind != "bc":
            continue
        for w, h, mips in ((1, 1, False), (7, 5, True), (32, 16, True), (19, 27, False)):
            y, x = np.mgrid[0:h, 0:w]
            img = np.stack([(x * 9) % 256, (y * 13) % 256, (x * y) % 256, 255 - (x + y) % 128], -1).astype(np.uint8)
            img = np.clip(img.astype(int) + nr.integers(-6, 7, img.shape), 0, 255).astype(np.uint8)
            flags = C.IOF_MIPMAPS if mips else 0
            recs.append(enc_rec(img, ddsfmt, flags, name=f"{name} {w}x{h}"))
            jobs.append((ddsfmt, img, w, h, max(w, h).bit_length() if mips else 1))
    res = runner.batch(recs)
    dec_recs = []
    for (ddsfmt, img, w, h, levels), r, rec in zip(jobs, res, recs):
        if r is None or r[0] != 0:
            suite.fail(f"{rec.name}: {'crash' if r is None else it.err_name(r[0])}")
            continue
        frames, end = bc_frames(r[1], ddsfmt, w, h, levels)
        suite.check(end == len(r[1]), f"{rec.name}: file length {len(r[1])} != {end}")
        bcf = WF[ddsfmt][2]
        for l, f, lw, lh, data in frames:
            exp = ref.ref_image(bcf, data, lw, lh)
            dec_recs.append(Rec(it.OP_DECEX, 0, decex(r[1], l), it.expect_decex(exp), f"{rec.name} mip {l}"))
        # PSNR of level 0 against the source
        exp0 = ref.ref_image(bcf, frames[0][4], w, h)
        if w * h >= 64:
            tgt = img
            if bcf in (4, 8):
                yy = lum(img)
                p = it.psnr(exp0[..., 2], yy)
            elif bcf in (5, 9):
                p = it.psnr(exp0[..., 1:3], img[..., 1:3])
            elif bcf in (6, 10):
                p = it.psnr(exp0[..., :3], tgt[..., :3])
            elif bcf == 1:
                vis = img[..., 3] >= 128
                p = it.psnr(exp0[vis][:, :3], img[vis][:, :3])
            else:
                p = it.psnr(exp0, img)
            suite.check(p > 18, f"{rec.name}: PSNR {p:.1f}")
    it.run_suite(runner, suite, dec_recs)


def area_down(prev, w, h):
    """Fant: area average of a straight BGRA image to w x h in premultiplied float."""
    ph, pw = prev.shape[:2]
    p = prev.astype(np.float64)
    a = p[..., 3:4] / 255.0
    pm = np.concatenate([p[..., :3] * a, p[..., 3:4]], -1)

    def weights(S, D):
        m = np.zeros((D, S))
        for i in range(D):
            lo, hi = i * S / D, (i + 1) * S / D
            for j in range(int(np.floor(lo)), int(np.ceil(hi))):
                m[i, j] = min(hi, j + 1) - max(lo, j)
            m[i] /= m[i].sum()
        return m
    my, mx = weights(ph, h), weights(pw, w)
    out = np.einsum("yi,ijc->yjc", my, pm)
    out = np.einsum("xj,yjc->yxc", mx, out)
    al = out[..., 3:4]
    col = np.where(al > 0, out[..., :3] / np.maximum(al, 1e-9) * 255.0, 0)
    return np.clip(np.rint(np.concatenate([col, al], -1)), 0, 255).astype(np.int64)


def suite_mips(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    recs = []
    sizes = ((64, 32), (17, 100), (1, 9), (9, 1), (4097, 1), (3, 5), (1, 1), (128, 128))
    for w, h in sizes:
        y, x = np.mgrid[0:h, 0:w]
        img = np.stack([(x * 5 + y) % 256, (y * 3) % 256, (x ^ y) % 256, np.full_like(x, 255)], -1).astype(np.uint8)
        if w * h > 16:
            img[..., 3] = np.where((x + y) % 7 == 0, 0, 255)
        levels = max(w, h).bit_length()
        recs.append((w, h, img, levels, enc_rec(img, 8, C.IOF_MIPMAPS, name=f"mips {w}x{h}")))
    res = runner.batch([r[4] for r in recs])
    dec = []
    for (w, h, img, levels, rec), r in zip(recs, res):
        if r is None or r[0] != 0:
            suite.fail(f"{rec.name}: encode failed")
            continue
        d = parse_header(r[1])
        suite.check(d["mips"] == levels, f"{rec.name}: {d['mips']} levels, expected {levels}")
        expected_len = 128 + sum(lw * lh * 4 for lw, lh in level_dims(w, h, levels))
        suite.check(len(r[1]) == expected_len, f"{rec.name}: length {len(r[1])} != {expected_len}")
        for l in range(levels):
            dec.append((w, h, l, Rec(it.OP_DECEX, 0, decex(r[1], l), None, f"{rec.name} level {l}")))
    out = runner.batch([d[3] for d in dec])
    levels_px = {}
    for (w, h, l, rec), r in zip(dec, out):
        if r is None or r[0] != 0:
            suite.fail(f"{rec.name}: decode failed")
            continue
        levels_px[(w, h, l)] = it.parse_decex(r[1])["px"]
    worst = 0
    for (w, h, l), px in levels_px.items():
        lw, lh = max(1, w >> l), max(1, h >> l)
        suite.check(px.shape[:2] == (lh, lw), f"level {l} of {w}x{h}: {px.shape} != {lh}x{lw}")
        if l == 0 or (w, h, l - 1) not in levels_px:
            continue
        exp = area_down(levels_px[(w, h, l - 1)], lw, lh)
        got = px.astype(np.int64)
        vis = exp[..., 3] > 8
        m = int(np.abs(got[..., 3] - exp[..., 3]).max())
        if vis.any():
            m = max(m, int(np.abs(got[vis][:, :3] - exp[vis][:, :3]).max()))
        worst = max(worst, m)
        suite.check(m <= 2, f"{w}x{h} level {l}: max |d| {m} vs the area average")
    suite.notes.append(f"Fant levels vs Python area average: max |d| {worst}")
    # other filters: close to Fant; sRGB: linear-light average of a black / white checker
    y, x = np.mgrid[0:64, 0:64]
    img = np.stack([128 + 100 * np.sin(x / 3.0), 128 + 100 * np.cos(y / 2.5), 128 + 90 * np.sin((x + y) / 5.0),
                    np.full(x.shape, 255.0)], -1)
    img = np.clip(np.rint(img + nr.normal(0, 6, img.shape)), 0, 255).astype(np.uint8)
    img[..., 3] = 255
    recs = [enc_rec(img, 8, C.IOF_MIPMAPS, mipf=m, name=f"filter {m}") for m in range(0, 7)]
    res = runner.batch(recs)
    fant = None
    for m, r in enumerate(res):
        rr = runner.one(it.OP_DECEX, 0, decex(r[1], 1))
        px = it.parse_decex(rr[1])["px"]
        if m == 0:
            fant = px
        else:
            p = it.psnr(px, fant)
            suite.notes.append(f"mip filter {m}: level 1 PSNR vs Fant {p:.1f} dB")
            suite.check(p > 25, f"mip filter {m}: level 1 far from Fant ({p:.1f} dB)")
    chk = np.zeros((16, 16, 4), np.uint8)
    chk[..., 3] = 255
    chk[(np.add.outer(np.arange(16), np.arange(16)) % 2) == 1, :3] = 255
    for srgb, lo, hi in ((0, 126, 130), (C.IOF_SRGB, 185, 190)):
        r = runner.one(it.OP_ENCODE, 0, opt(8, C.IOF_MIPMAPS | srgb) + it.surface_input(chk, C.FMT_ARGB))
        rr = runner.one(it.OP_DECEX, 0, decex(r[1], 1))
        v = float(it.parse_decex(rr[1])["px"][..., :3].mean())
        suite.check(lo <= v <= hi, f"checker level 1 mean {v:.1f} not in {lo}..{hi} (sRGB={bool(srgb)})")
        suite.notes.append(f"black / white checker, level 1 mean {v:.1f} ({'linear light' if srgb else 'gamma space'})")


CROSS = ref.CROSS


def suite_cube(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    recs = []
    for ddsfmt in (8, 1, 7, 14):
        for s, mips in ((8, True), (5, False), (1, True)):
            img = rand_img(nr, 4 * s, 3 * s, alpha=ddsfmt != 1)
            recs.append((ddsfmt, s, mips, img, enc_rec(img, ddsfmt, C.IOF_CUBE_CROSS | (C.IOF_MIPMAPS if mips else 0),
                                                        name=f"cube {WF[ddsfmt][0]} {s} mips={mips}")))
    res = runner.batch([r[4] for r in recs])
    dec = []
    for (ddsfmt, s, mips, img, rec), r in zip(recs, res):
        if r is None or r[0] != 0:
            suite.fail(f"{rec.name}: {'crash' if r is None else it.err_name(r[0])}")
            continue
        d = parse_header(r[1])
        suite.check(d["caps2"] == 0xFE00 and d["caps"] & 0x8, f"{rec.name}: caps {d['caps']:#x} / {d['caps2']:#x}")
        levels = s.bit_length() if mips else 1
        for l in range(levels):
            ls = max(1, s >> l)
            if WF[ddsfmt][1] == "raw" and l == 0:
                exp = np.zeros_like(img)
                faces = {k: img[cy * s:(cy + 1) * s, cx * s:(cx + 1) * s] for k, (cx, cy) in CROSS.items()}
                for k, (cx, cy) in CROSS.items():
                    exp[cy * s:(cy + 1) * s, cx * s:(cx + 1) * s] = decoded_raw(WF[ddsfmt][2], faces[k])
                dec.append(Rec(it.OP_DECEX, 0, decex(r[1], l), it.expect_decex(exp), f"{rec.name} level {l}"))
            else:
                bcf = WF[ddsfmt][2]
                fr, _ = bc_frames(r[1], ddsfmt, s, s, levels, 6) if WF[ddsfmt][1] == "bc" else (None, None)

                def chk(res_, out, fr=fr, l=l, ls=ls, bcf=bcf, ddsfmt=ddsfmt):
                    if res_ != 0:
                        return f"error {it.err_name(res_)}"
                    px = it.parse_decex(out)["px"]
                    if px.shape[:2] != (3 * ls, 4 * ls):
                        return f"cross {px.shape}"
                    if fr is None:
                        return None
                    for (lv, f, lw, lh, data) in fr:
                        if lv != l:
                            continue
                        cx, cy = CROSS[f]
                        e = ref.ref_image(bcf, data, lw, lh)
                        if not np.array_equal(px[cy * ls:(cy + 1) * ls, cx * ls:(cx + 1) * ls], e):
                            return f"face {f} differs"
                    return None
                dec.append(Rec(it.OP_DECEX, 0, decex(r[1], l), chk, f"{rec.name} level {l}"))
    it.run_suite(runner, suite, dec)
    img = rand_img(nr, 20, 16)
    it.run_suite(runner, suite, [
        enc_rec(img, 8, C.IOF_CUBE_CROSS, chk=it.expect_err(C.ERR_DIMS), name="cross 20x16"),
        enc_rec(rand_img(nr, 18, 12), 8, C.IOF_CUBE_CROSS, chk=it.expect_err(C.ERR_DIMS), name="cross 18x12"),
        enc_rec(rand_img(nr, 12, 12), 1, C.IOF_CUBE_CROSS, chk=it.expect_err(C.ERR_DIMS), name="cross 12x12")])


def run_wic(suite, runner, rng):
    if runner.target != "win":
        return
    nr = np.random.default_rng(rng.randrange(1 << 30))
    work = Path(tempfile.mkdtemp(prefix="ddswwic_", dir=runner.work))
    wic = it.Wic(work)
    jobs = []
    # WIC's DDS codec decodes BC1..BC3 only (not the DX10 sRGB twins, not uncompressed layouts)
    for ddsfmt, srgb in ((1, 0), (2, 0), (3, 0), (1, C.IOF_SRGB), (8, 0)):
        for w, h in ((32, 32), (20, 12)):
            y, x = np.mgrid[0:h, 0:w]
            img = np.stack([(x * 8) % 256, (y * 20) % 256, (x * y) % 256, np.where(x % 5 == 0, 60, 255)], -1).astype(np.uint8)
            img = np.clip(img.astype(int) + nr.integers(-9, 10, img.shape), 0, 255).astype(np.uint8)
            r = runner.one(it.OP_ENCODE, 0, opt(ddsfmt, srgb) + it.surface_input(img, C.FMT_ARGB))
            if r[0] != 0:
                suite.fail(f"encode {ddsfmt}: {it.err_name(r[0])}")
                continue
            p = work / f"f{ddsfmt}_{srgb}_{w}x{h}.dds"
            p.write_bytes(r[1])
            jobs.append((f"{WF[ddsfmt][0]}{' sRGB' if srgb else ''} {w}x{h}", r[1], wic.decode(p)))
    wic.run()
    recs = []
    for name, data, ji in jobs:
        res = wic.result(ji)
        if res is None:
            if "sRGB" in name or "BGRA8" in name:
                suite.notes.append(f"WIC does not decode {name} (as for any such file)")
            else:
                suite.fail(f"WIC cannot decode {name}")
            continue
        wpx = res[3]
        vis = wpx[..., 3] > 0

        def chk(r, out, wpx=wpx, vis=vis, name=name):
            if r != 0:
                return f"error {it.err_name(r)}"
            px = it.parse_decex(out)["px"]
            if not np.array_equal(px[..., 3], wpx[..., 3]):
                return "alpha differs from WIC"
            if vis.any() and not np.array_equal(px[vis][:, :3], wpx[vis][:, :3]):
                return f"colours differ from WIC (max {it.max_abs(px[vis][:, :3], wpx[vis][:, :3])})"
            return None
        recs.append(Rec(it.OP_DECEX, 0, decex(data), chk, f"wic {name}"))
    it.run_suite(runner, suite, recs)
    suite.notes.append(f"{len(recs)} files decoded by WIC identically to our reader")


FFMPEG = [(1, 2), (2, 2), (3, 2), (4, 2), (17, 2), (5, 2), (18, 2), (8, 0), (9, 0), (10, 0), (13, 4), (11, 4),
          (14, 0), (15, 0), (21, 0)]                   # (ffmpeg has no A4R4G4B4)


def run_ffmpeg(suite, runner, rng):
    if not it.have_exe("ffmpeg"):
        suite.notes.append("ffmpeg not found -- skipped")
        return
    nr = np.random.default_rng(rng.randrange(1 << 30))
    work = Path(tempfile.mkdtemp(prefix="ddswff_", dir=runner.work))
    recs, notes = [], []
    for ddsfmt, tol in FFMPEG:
        w, h = 36, 20
        y, x = np.mgrid[0:h, 0:w]
        img = np.stack([(x * 7) % 256, (y * 11) % 256, (x * y) % 256, 255 - (x * 3) % 200], -1).astype(np.uint8)
        img = np.clip(img.astype(int) + nr.integers(-5, 6, img.shape), 0, 255).astype(np.uint8)
        r = runner.one(it.OP_ENCODE, 0, opt(ddsfmt) + it.surface_input(img, C.FMT_ARGB))
        if r[0] != 0:
            suite.fail(f"encode {WF[ddsfmt][0]}: {it.err_name(r[0])}")
            continue
        p = work / f"{WF[ddsfmt][0]}.dds"
        p.write_bytes(r[1])
        fref = it.ffmpeg_decode(p)
        name = WF[ddsfmt][0]
        if fref is None:
            suite.fail(f"ffmpeg cannot decode our {name}")
            continue
        chans = [0, 1, 2, 3]
        if ddsfmt in (5, 18):
            chans = [1, 2]                      # ffmpeg rebuilds blue (normal map z)
        if ddsfmt == 15:
            fref = fref.copy()
            fref[..., 3] = fref[..., 0]         # ffmpeg shows A8 as grey
            chans = [3]
        if ddsfmt == 11:
            chans = [0, 1, 2]                   # ffmpeg ignores the alpha bit (rgb555)
        if ddsfmt in (4, 17, 14):
            chans = [0, 1, 2]

        def chk(res, out, fref=fref, tol=tol, chans=chans, name=name):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            m = it.max_abs(d["px"][..., chans], fref[..., chans])
            notes.append(f"{name}: max |d| vs ffmpeg {m}")
            return None if m <= tol else f"max |d| {m} > {tol}"
        recs.append(Rec(it.OP_DECEX, 0, decex(r[1]), chk, f"ffmpeg {name}"))
    it.run_suite(runner, suite, recs)
    suite.notes += notes


def suite_errors(runner, suite, rng):
    nr = np.random.default_rng(rng.randrange(1 << 30))
    img = rand_img(nr, 8, 8)
    E = it.expect_err
    recs = [enc_rec(img, 22, chk=E(C.ERR_UNSUPPORTED), name="IO_DDSFMT 22"),
            enc_rec(img, 1000, chk=E(C.ERR_UNSUPPORTED), name="IO_DDSFMT 1000"),
            enc_rec(img, 0, chk=lambda r, o: None if r == 0 and parse_header(o)["pf"][1] == fourcc("DXT1") else
                    f"default not BC1 ({it.err_name(r)})", name="IO_DDSFMT 0 = BC1")]
    for fmt in (1, 7, 8):
        synth = opt(fmt, C.IOF_MIPMAPS) + struct.pack("<4I", 64, 64, 0 | 256, 1)
        recs.append(Rec(OP_ENC_SYN, 0, synth, E(C.ERR_CANCEL), f"cancel {WF[fmt][0]}"))
    it.run_suite(runner, suite, recs)


def suite_big(runner, suite, rng, full):
    jobs = [((1, C.IOF_MIPMAPS, 4097, 1)), ((7, C.IOF_MIPMAPS, 1, 4097)), ((7, C.IOF_MIPMAPS, 4097, 1)),
            ((8, C.IOF_MIPMAPS, 3, 4097))]
    for f in WF:
        jobs.append((f, C.IOF_MIPMAPS, 1, 1))
    if full:
        jobs += [(1, C.IOF_MIPMAPS, 16384, 16384), (8, C.IOF_MIPMAPS, 8192, 8192), (7, 0, 8192, 8192)]
    recs = []
    for ddsfmt, flags, w, h in jobs:
        levels = max(w, h).bit_length() if flags & C.IOF_MIPMAPS else 1
        exp = expected_header(ddsfmt, w, h, levels, False, False)
        exp_len = (148 if "dx10" in exp else 128) + sum(level_bytes(ddsfmt, lw, lh) for lw, lh in level_dims(w, h, levels))

        def chk(res, out, exp_len=exp_len):
            if res != 0:
                return f"error {it.err_name(res)}"
            n, us, hsh = struct.unpack_from("<QII", out)
            return None if n == exp_len else f"length {n} != {exp_len}"
        recs.append(Rec(OP_ENC_SYN, 0, opt(ddsfmt, flags) + struct.pack("<4I", w, h, 0, 1), chk,
                        f"{WF[ddsfmt][0]} {w}x{h} levels {levels}"))
    res = it.run_suite(runner, suite, recs)
    for r, rec in zip(res, recs):
        if r is not None and r[0] == 0 and ("16384" in rec.name or "8192" in rec.name):
            n, us, _ = struct.unpack_from("<QII", r[1])
            suite.notes.append(f"{rec.name}: {n / 1e6:.1f} MB in {us / 1e6:.2f} s")


def run_perf(runner, suite, rng):
    for ddsfmt, eff in ((1, 0), (3, 0), (7, 1), (7, 0), (8, 0)):
        rec = Rec(OP_ENC_SYN, 0, opt(ddsfmt, C.IOF_MIPMAPS, effort=eff) + struct.pack("<4I", 4096, 4096, 0, 1), None, "p")
        r = runner.batch([rec])[0]
        if r is None or r[0] != 0:
            suite.fail(f"perf {WF[ddsfmt][0]}: failed")
            continue
        n, us, _ = struct.unpack_from("<QII", r[1])
        lvl = {0: "normal", 1: "fast"}[eff]
        suite.notes.append(f"DDS {WF[ddsfmt][0]} {lvl} 4096^2 + mips (pool): {us / 1e6:.2f} s, {n / 1e6:.1f} MB")


def main():
    ap = it.argparser(__doc__, SUITES)
    ap.add_argument("--full", action="store_true", help="16384^2 / 8192^2 images in the big suite")
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
        runner = it.Runner(target, out, work_name="ddsw")
        for name, fn in (("header", suite_header), ("raw", suite_raw), ("bc", suite_bc), ("mips", suite_mips),
                         ("cube", suite_cube), ("errors", suite_errors)):
            if want(name):
                s = it.Suite(name, target)
                fn(runner, s, random.Random(a.seed + len(name)))
                suites.append(s)
        if want("wic") and not a.no_ext and target == "win":
            s = it.Suite("wic", target)
            run_wic(s, runner, random.Random(a.seed + 20))
            suites.append(s)
        if want("ffmpeg") and not a.no_ext:
            s = it.Suite("ffmpeg", target)
            run_ffmpeg(s, runner, random.Random(a.seed + 21))
            suites.append(s)
        if want("big"):
            s = it.Suite("big", target)
            suite_big(runner, s, random.Random(a.seed + 22), a.full)
            suites.append(s)
        if want("perf"):
            s = it.Suite("perf", target)
            run_perf(runner, s, random.Random(a.seed + 23))
            suites.append(s)
    ok = it.report(suites, "dds_write_check")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
