#!/usr/bin/env python3
"""Checks of the codec infrastructure (phase P0): imgio.s (dispatch, detection, format
table), imgmeta.s (metadata records, pixel format conversion, orientation, sample depth
converters), exif.s, icc.s, gzip.s, deflate_raw / inflate_raw_ex, imgcore.s (metadata
of the core formats).  Runs apps/codectest ops 240..259 (meta_ops.s) on every target.

Suites
  gzip     gzip_decode of Python gzip members (levels, FEXTRA / FNAME / FCOMMENT / FHCRC /
           FTEXT, multi-member streams, trailing data) -> identical bytes + consumed length;
           truncations / corrupt header, CRC, ISIZE -> the documented error codes; gzip_encode
           -> Python gzip.decompress + header layout; deflate_raw -> zlib raw; inflate_raw_ex
           consumed length before trailing data
  exif     exif_read (II / MM / BigTIFF, orientation 0..9, X/YResolution with inch / cm /
           none / default unit, ColorSpace, IFD1, cyclic IFD chains, truncation, bad headers),
           exif_copy_orient1
  icc      icc_check, icc_jpeg_segments (1 / 2 / 4 segments, 255-segment limit),
           icc_jpeg_collect (in order, shuffled, unrelated APP2, malformed sequences),
           IMGD_ICC through JPEG (APP2), PNG (iCCP) and BMP (V5 embedded profile)
  detect   img_detect for every magic (TIFF / BigTIFF / JPEG XR / ISOBMFF brands and primary
           item types / JPEG XL / PDN / gzip / BMP header sizes / TGA hint and heuristic;
           the heuristic only with the codecs registered, --core-only without)
  probe    IMGMETA from img_probe for PNG (IHDR, tRNS, pHYs, gAMA, sRGB, cICP, acTL, fcTL),
           JPEG (SOF, JFIF / EXIF density, EXIF orientation, Adobe, progressive), GIF (frames,
           loops, delay, transparency), BMP (depth, alpha mask, dpi)
  fmtinfo  img_format_info (names, extension lists, MIME, caps, IOM, registration with and
           without --core-only), img_format_from_path, img_filter_list
  decex    img_decode_ex against the classic img_decode (same pixels, same error codes),
           IMGD_STRAIGHT (unpremultiplied from the premultiplied decode -- the classic
           decoders have no straight path yet), IMGD_PROBE, ID_MAXPX -> ERR_LIMIT, cancel,
           frame > 0, unknown / short data, stub formats, EXIF orientation (JPEG: default ==
           classic, IMGD_NO_ORIENT == the file without EXIF; PNG eXIf applied by the
           dispatcher), EXIF / XMP / ICC blobs, no IMGMETA
  pixels   img_convert / img_surf_convert / img_get_row exhaustively over (c, a), XRGB and A8
           sources; row converters (all 65536 u16 LE / BE, all halves, float specials, sRGB);
           img_orient 1..8
  encode   img_encode PNG / JPEG / BMP / GIF from PARGB / ARGB / XRGB / A8, options,
           unsupported formats; --convert-ex (img_load_ex + img_save_opt)
  fuzz     mutated inputs through decode_ex (all metadata flags), probe, exif_read,
           icc_jpeg_collect, gzip_decode -> any result, no crash (inputs end at a guard page)

  python tools/imgmeta_check.py --target both --out build/c0
  python tools/imgmeta_check.py --target win --only gzip,exif --no-build
"""

import gzip
import random
import struct
import subprocess
import sys
import zlib
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402

C = it.C
Rec = it.Rec
SUITES = "gzip,exif,icc,detect,probe,fmtinfo,decex,pixels,encode,fuzz"


# ================================================================== helpers

def imgopt(fmt, quality=0, flags=0, bitdepth=0, effort=0):
    return struct.pack("<12IQQ", fmt, quality, flags, bitdepth, 0, 0, 0, 0, effort, 0, 0, 0, 0, 0)


def encode_file(runner, fmt, px, sfmt, **kw):
    """Encode a surface through op 252 (img_encode) -> file bytes (raises on error)."""
    res, out = runner.one(it.OP_ENCODE, 0, imgopt(fmt, **kw) + it.surface_input(px, sfmt))
    if res != 0:
        raise RuntimeError(f"img_encode fmt {fmt}: {it.err_name(res)}")
    return bytes(out)


def old_decode(runner, data):
    res, out = runner.one(it.OP_IMGDEC, 0, data)
    if res != 0:
        return res, None
    w, h = struct.unpack_from("<II", out, 0)
    return 0, np.frombuffer(out, np.uint8, offset=8).reshape(h, w, 4).copy()


def smooth_bgra(rng, w, h, alpha="opaque"):
    r = np.random.default_rng(rng.randrange(1 << 30))
    y, x = np.mgrid[0:h, 0:w]
    px = np.zeros((h, w, 4), np.uint8)
    px[..., 0] = (x * 255 // max(1, w - 1))
    px[..., 1] = (y * 255 // max(1, h - 1))
    px[..., 2] = ((x + y) * 127 // max(1, w + h)) + r.integers(0, 8, (h, w))
    px[..., 3] = 255
    if alpha != "opaque":
        px[..., 3] = r.integers(0, 256, (h, w))
    return px


def jpeg_strip_app0(j):
    if j[2:4] == b"\xff\xe0":
        n = struct.unpack(">H", j[4:6])[0]
        return j[:2] + j[4 + n:]
    return j


def xmp_app1(xml):
    return it.jpeg_seg(0xE1, b"http://ns.adobe.com/xap/1.0/\0" + xml)


def icc_profile(extra=200, version=4, tags=3, sig=b"acsp", size_delta=0, bad_tag=False):
    """Synthetic ICC profile: 128-byte header + tag table + tag data."""
    n = tags
    data_off = 132 + 12 * n
    body = bytearray()
    table = bytearray()
    for i in range(n):
        payload = bytes([(i * 37 + k) & 0xFF for k in range(20 + i * 8)])
        off = data_off + len(body)
        size = len(payload)
        if bad_tag and i == n - 1:
            size = 1 << 20
        table += struct.pack(">4sII", [b"desc", b"wtpt", b"cprt", b"rXYZ", b"gXYZ"][i % 5], off, size)
        body += payload + b"\0" * ((-len(payload)) & 3)
    body += bytes((k * 7) & 0xFF for k in range(extra))
    total = data_off + len(body)
    hdr = bytearray(128)
    struct.pack_into(">I", hdr, 0, total + size_delta)
    hdr[4:8] = b"ASMG"
    hdr[8] = version
    hdr[9] = 0x30
    hdr[12:16] = b"mntr"
    hdr[16:20] = b"RGB "
    hdr[20:24] = b"XYZ "
    hdr[36:40] = sig
    return bytes(hdr) + struct.pack(">I", n) + bytes(table) + bytes(body)


def icc_segments_py(icc):
    segs = []
    n = -(-len(icc) // 65519)
    for i in range(n):
        chunk = icc[i * 65519:(i + 1) * 65519]
        segs.append(it.jpeg_seg(0xE2, b"ICC_PROFILE\0" + bytes([i + 1, n]) + chunk))
    return segs


def gif_lzw_tiny(indices, mcs=2):
    """LZW data (min code size mcs) that keeps the code width at mcs + 1: a clear code
    before every two literals."""
    clear, end = 1 << mcs, (1 << mcs) + 1
    width = mcs + 1
    codes = []
    for i, v in enumerate(indices):
        if i % 2 == 0:
            codes.append(clear)
        codes.append(v)
    codes.append(end)
    acc, nb, out = 0, 0, bytearray()
    for c in codes:
        acc |= c << nb
        nb += width
        while nb >= 8:
            out.append(acc & 0xFF)
            acc >>= 8
            nb -= 8
    if nb:
        out.append(acc & 0xFF)
    blocks = bytearray([mcs])
    for i in range(0, len(out), 255):
        blocks.append(len(out[i:i + 255]))
        blocks += out[i:i + 255]
    blocks.append(0)
    return bytes(blocks)


def gif_anim(frames, w, h, loops=3, delay=20, transp=True):
    """GIF89a with a 4-colour global table, NETSCAPE loop count and one GCE per frame."""
    out = bytearray(b"GIF89a" + struct.pack("<HHBBB", w, h, 0x81, 0, 0))
    out += bytes([0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255])
    out += b"\x21\xff\x0bNETSCAPE2.0\x03\x01" + struct.pack("<H", loops) + b"\x00"
    for f in frames:
        out += b"\x21\xf9\x04" + bytes([0x01 if transp else 0]) + struct.pack("<H", delay) + b"\x03\x00"
        out += b"\x2c" + struct.pack("<HHHHB", 0, 0, w, h, 0)
        out += gif_lzw_tiny(f)
    out += b"\x3b"
    return bytes(out)


def bmp_v5(straight_bgra, icc=None, ppm=2835):
    """32-bit BITMAPV5HEADER BI_BITFIELDS bottom-up BMP with an alpha mask, optionally with an
    embedded ICC profile after the pixels."""
    h, w = straight_bgra.shape[:2]
    pix = np.ascontiguousarray(straight_bgra[::-1]).tobytes()
    off = 14 + 124
    prof_off = 124 + len(pix)
    hdr = struct.pack("<IiiHHIIiiII", 124, w, h, 1, 32, 3, len(pix), ppm, ppm, 0, 0)
    hdr += struct.pack("<IIII", 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000)
    hdr += struct.pack("<I", 0x4D424544 if icc else 0x73524742) + b"\0" * 36 + b"\0" * 12
    hdr += struct.pack("<IIII", 4, prof_off if icc else 0, len(icc) if icc else 0, 0)
    assert len(hdr) == 124
    body = hdr + pix + (icc or b"")
    return b"BM" + struct.pack("<IHHI", 14 + len(body), 0, 0, off) + body


def tga_header(w, h, typ=2, depth=32, desc=8, cmap=(0, 0, 0, 0), idlen=0):
    ct, first, n, bits = cmap
    return struct.pack("<BBBHHBHHHHBB", idlen, ct, typ, first, n, bits, 0, 0, w, h, depth, desc)


def ftyp_box(major, compat):
    return it.ftyp(major, 0, compat)


def heif_meta(primary, items, pitm_v=0, infe_v=2):
    hdlr = it.bmff_fullbox(b"hdlr", 0, 0, b"\0" * 4 + b"pict" + b"\0" * 12 + b"\0")
    pitm = it.bmff_fullbox(b"pitm", pitm_v, 0, struct.pack(">H" if pitm_v == 0 else ">I", primary))
    infes = b""
    for iid, typ in items:
        if infe_v == 2:
            infes += it.bmff_fullbox(b"infe", 2, 0, struct.pack(">HH", iid, 0) + typ + b"\0")
        else:
            infes += it.bmff_fullbox(b"infe", 3, 0, struct.pack(">IH", iid, 0) + typ + b"\0")
    iinf = it.bmff_fullbox(b"iinf", 0, 0, struct.pack(">H", len(items)) + infes)
    return it.bmff_fullbox(b"meta", 0, 0, hdlr + iinf + pitm)


def dpi16_ppm(ppm):
    return min(0xFFFFFFFF, (ppm * 254 * 65536 + 5000) // 10000)


def exif_dpi_ref(num, den, unit):
    if den == 0 or unit not in (2, 3):
        return 0
    v = num << 16
    if unit == 3:
        v *= 254
        den *= 100
    return min(0xFFFFFFFF, (v + den // 2) // den)


# ================================================================== gzip

def gz_member(data, flags=0, extra=b"", name=b"", comment=b"", level=6, bad_hcrc=False,
              bad_crc=False, bad_isize=False, xfl=0, os_=3):
    hdr = bytearray(b"\x1f\x8b\x08" + bytes([flags]) + b"\0\0\0\0" + bytes([xfl, os_]))
    if flags & 4:
        hdr += struct.pack("<H", len(extra)) + extra
    if flags & 8:
        hdr += name + b"\0"
    if flags & 16:
        hdr += comment + b"\0"
    if flags & 2:
        c = zlib.crc32(bytes(hdr)) & 0xFFFF
        hdr += struct.pack("<H", c ^ (1 if bad_hcrc else 0))
    co = zlib.compressobj(level, zlib.DEFLATED, -15)
    body = co.compress(data) + co.flush()
    crc = zlib.crc32(data) ^ (1 if bad_crc else 0)
    isz = (len(data) + (1 if bad_isize else 0)) & 0xFFFFFFFF
    return bytes(hdr) + body + struct.pack("<II", crc & 0xFFFFFFFF, isz)


def gz_expect(data, consumed):
    def chk(res, out):
        if res != len(data):
            return f"result {it.err_name(res) if res < 0 else res}, expected {len(data)}"
        c = struct.unpack_from("<Q", out, 0)[0]
        if c != consumed:
            return f"consumed {c}, expected {consumed}"
        if bytes(out[8:]) != data:
            return "output differs"
        return None
    return chk


def suite_gzip(rng):
    recs = []
    sets = {"empty": b"", "one": b"a", "text": (b"The quick brown fox jumps over the lazy dog. " * 300),
            "rand": bytes(rng.randrange(256) for _ in range(70000)), "zeros": b"\0" * 200000}
    for name, d in sets.items():
        cap = len(d) + 64
        for lv in (0, 1, 6, 9):
            g = gzip.compress(d, lv, mtime=0)
            recs.append(Rec(it.OP_GZDEC, cap, g, gz_expect(d, len(g)), f"py-gzip {name} L{lv}"))
        g = gz_member(d)
        recs.append(Rec(it.OP_GZDEC, len(d), g, gz_expect(d, len(g)), f"exact cap {name}"))
        if d:
            recs.append(Rec(it.OP_GZDEC, len(d) - 1, g, it.expect_err(C.ERR_OVERFLOW), f"small cap {name}"))
    d = sets["text"]
    for fl in range(32):
        g = gz_member(d, flags=fl, extra=b"\x01\x02AB\x00\x00" if fl & 4 else b"",
                      name=b"layer0.bin" if fl & 8 else b"", comment=b"jfgui" if fl & 16 else b"")
        recs.append(Rec(it.OP_GZDEC, len(d), g, gz_expect(d, len(g)), f"flags {fl:#x}"))
    g1, g2 = gz_member(b"first member"), gz_member(b"second")
    recs.append(Rec(it.OP_GZDEC, 100, g1 + g2, gz_expect(b"first member", len(g1)), "multi-member"))
    recs.append(Rec(it.OP_GZDEC, 100, g1 + b"GARBAGE", gz_expect(b"first member", len(g1)), "trailing data"))
    g = gz_member(d, flags=2 | 8, name=b"x")
    recs.append(Rec(it.OP_GZDEC, len(d), gz_member(d, flags=2, bad_hcrc=True), it.expect_err(C.ERR_CHECKSUM), "bad FHCRC"))
    recs.append(Rec(it.OP_GZDEC, len(d), gz_member(d, bad_crc=True), it.expect_err(C.ERR_CHECKSUM), "bad CRC32"))
    recs.append(Rec(it.OP_GZDEC, len(d) + 8, gz_member(d, bad_isize=True), it.expect_err(C.ERR_CHECKSUM), "bad ISIZE"))
    bad = bytearray(g)
    bad[0] = 0x1e
    recs.append(Rec(it.OP_GZDEC, len(d), bytes(bad), it.expect_err(C.ERR_HDR), "bad ID1"))
    bad = bytearray(g)
    bad[2] = 7
    recs.append(Rec(it.OP_GZDEC, len(d), bytes(bad), it.expect_err(C.ERR_HDR), "CM 7"))
    bad = bytearray(g)
    bad[3] |= 0x20
    recs.append(Rec(it.OP_GZDEC, len(d), bytes(bad), it.expect_err(C.ERR_HDR), "reserved flag"))
    # truncations: header / name / trailer -> TRUNC, inside the deflate data: any error
    hdr_end = 10 + 2   # "x\0"
    for cut in range(0, len(g), max(1, len(g) // 60)):
        if cut < hdr_end or cut > len(g) - 8:
            exp = it.expect_err(C.ERR_TRUNC)
        else:
            exp = (lambda res, out: None if res < 0 else f"accepted a truncated member ({res})")
        recs.append(Rec(it.OP_GZDEC, len(d), g[:cut], exp, f"truncated at {cut}"))
    for cut in range(len(g) - 8, len(g)):
        recs.append(Rec(it.OP_GZDEC, len(d), g[:cut], it.expect_err(C.ERR_TRUNC), f"trailer cut {cut}"))
    # encoder
    for name, dd in sets.items():
        for lv in (-1, 0, 1, 6, 9):
            def chk(res, out, dd=dd, lv=lv):
                if res != 0:
                    return f"error {it.err_name(res)}"
                out = bytes(out)
                if out[:4] != b"\x1f\x8b\x08\x00" or out[4:8] != b"\0\0\0\0" or out[9] != 255:
                    return f"header {out[:10].hex()}"
                xfl = 2 if lv == 9 else 4 if lv == 1 else 0
                if out[8] != xfl:
                    return f"XFL {out[8]} != {xfl}"
                if gzip.decompress(out) != dd:
                    return "Python gzip.decompress differs"
                return None
            recs.append(Rec(it.OP_GZENC, lv, dd, chk, f"gzip_encode {name} L{lv}"))
        for lv in (0, 1, 5, 9):
            def chk2(res, out, dd=dd):
                if res != 0:
                    return f"error {it.err_name(res)}"
                return None if zlib.decompress(bytes(out), -15) == dd else "zlib raw decode differs"
            recs.append(Rec(it.OP_DEFRAW, lv, dd, chk2, f"deflate_raw {name} L{lv}"))
        co = zlib.compressobj(6, zlib.DEFLATED, -15)
        raw = co.compress(dd) + co.flush()
        recs.append(Rec(it.OP_INFRAW, len(dd) + 16, raw + b"TRAILING DATA", gz_expect(dd, len(raw)),
                        f"inflate_raw_ex {name}"))
    return recs


# ================================================================== exif

def exi_expect(orient, dpix=0, dpiy=0, unit=0, cspace=0, result=0, orioff_set=None):
    def chk(res, out):
        if res != result:
            return f"result {it.err_name(res)} != {it.err_name(result)}"
        o, u, dx, dy, cs, off, eo = struct.unpack_from("<7I", out, 0)
        got = dict(orient=o, unit=u, dpix=dx, dpiy=dy, cspace=cs)
        exp = dict(orient=orient, unit=unit, dpix=dpix, dpiy=dpiy, cspace=cspace)
        if got != exp:
            return f"got {got}, expected {exp}"
        if eo != orient:
            return f"exif_orientation {eo} != {orient}"
        if orioff_set is not None and (off != 0) != orioff_set:
            return f"orientation offset {off}"
        return None
    return chk


def suite_exif(rng):
    recs = []
    for bo in ("<", ">"):
        for o in range(0, 10):
            b = it.exif_blob(orient=o, bo=bo)
            recs.append(Rec(it.OP_EXIFREAD, 0, b, exi_expect(o if 1 <= o <= 8 else 1, orioff_set=True),
                            f"orient {o} {bo}"))
        for num, den, unit in ((300, 1, 2), (72, 1, None), (7200, 100, 2), (118, 1, 3), (1, 3, 3),
                               (96, 1, 1), (300, 0, 2), (0xFFFFFFFF, 1, 2), (5, 7, 2)):
            b = it.exif_blob(orient=1, xres=(num, den), yres=(num * 2 % (1 << 32), den), unit=unit, bo=bo)
            u = 2 if unit is None else unit
            recs.append(Rec(it.OP_EXIFREAD, 0, b,
                            exi_expect(1, exif_dpi_ref(num, den, u), exif_dpi_ref(num * 2 % (1 << 32), den, u),
                                       unit or 0), f"res {num}/{den} unit {unit} {bo}"))
        for cs in (1, 0xFFFF):
            b = it.exif_blob(orient=3, colorspace=cs, extra_ifd1=True, bo=bo)
            recs.append(Rec(it.OP_EXIFREAD, 0, b, exi_expect(3, cspace=cs), f"colorspace {cs} {bo}"))
        b = it.exif_blob(orient=8, colorspace=1, cycle=True, bo=bo)
        recs.append(Rec(it.OP_EXIFREAD, 0, b, exi_expect(8, cspace=1), f"cyclic chain {bo}"))
        big = it.tiff_bytes([{"entries": [(0x0112, 3, [5]), (0x011A, 5, [(200, 1)]), (0x0128, 3, [2])],
                              "next": None}], bo=bo, big=True)
        recs.append(Rec(it.OP_EXIFREAD, 0, big, exi_expect(5, dpix=200 << 16, unit=2), f"BigTIFF {bo}"))
        # a long chain: 2000 IFDs -> stops at the limit, still result 0
        ifds = [{"entries": [(0x0112, 3, [6])], "next": 1}] + \
               [{"entries": [(0x0100, 3, [i])], "next": i + 2 if i < 1998 else None} for i in range(1999)]
        recs.append(Rec(it.OP_EXIFREAD, 0, it.tiff_bytes(ifds, bo=bo), exi_expect(6), f"2000-IFD chain {bo}"))
        # orientation with the wrong type -> ignored
        b = it.tiff_bytes([{"entries": [(0x0112, 4, [6])], "next": None}], bo=bo)
        recs.append(Rec(it.OP_EXIFREAD, 0, b, exi_expect(1, orioff_set=False), f"orient LONG {bo}"))
        # copy with orientation 1
        for o in (1, 3, 6, 8):
            src = it.exif_blob(orient=o, xres=(300, 1), unit=2, colorspace=1, bo=bo)
            exp = it.exif_blob(orient=1, xres=(300, 1), unit=2, colorspace=1, bo=bo)
            recs.append(Rec(it.OP_EXIF1, 0, src, it.expect_bytes(exp), f"copy_orient1 {o} {bo}"))
        nob = it.exif_blob(xres=(72, 1), bo=bo)
        recs.append(Rec(it.OP_EXIF1, 0, nob, it.expect_bytes(nob), f"copy_orient1 without tag {bo}"))
    recs.append(Rec(it.OP_EXIFREAD, 0, b"XX*\0\x08\0\0\0\0\0", exi_expect(1, result=C.ERR_HDR), "bad byte order"))
    recs.append(Rec(it.OP_EXIFREAD, 0, b"II+\0\x04\0\0\0\0\0\0\0\0\0\0\0", exi_expect(1, result=C.ERR_HDR),
                    "BigTIFF bad offset size"))
    recs.append(Rec(it.OP_EXIFREAD, 0, b"II*\0\x08\0", exi_expect(1, result=C.ERR_TRUNC), "short"))
    recs.append(Rec(it.OP_EXIFREAD, 0, b"II*\0\xff\0\0\0", exi_expect(1, result=C.ERR_TRUNC), "IFD0 outside"))
    recs.append(Rec(it.OP_EXIFREAD, 0, b"", exi_expect(1, result=C.ERR_TRUNC), "empty"))
    # IFD count larger than the data: entries that fit are still used
    b = bytearray(it.exif_blob(orient=7))
    struct.pack_into("<H", b, 8, 500)
    recs.append(Rec(it.OP_EXIFREAD, 0, bytes(b), exi_expect(7), "count clamped"))
    recs.append(Rec(it.OP_EXIF1, 0, b"", it.expect_bytes(b""), "copy empty"))
    return recs


# ================================================================== icc

def suite_icc(rng, runner):
    recs = []
    prof = icc_profile()
    recs.append(Rec(it.OP_ICCCHK, 0, prof, it.expect_err(len(prof)), "valid v4"))
    recs.append(Rec(it.OP_ICCCHK, 0, icc_profile(version=2), it.expect_err(len(prof)), "valid v2"))
    recs.append(Rec(it.OP_ICCCHK, 0, prof + b"xyz", it.expect_err(len(prof)), "trailing bytes"))
    recs.append(Rec(it.OP_ICCCHK, 0, prof[:131], it.expect_err(C.ERR_TRUNC), "short"))
    recs.append(Rec(it.OP_ICCCHK, 0, prof[:-1], it.expect_err(C.ERR_TRUNC), "declared size > len"))
    recs.append(Rec(it.OP_ICCCHK, 0, icc_profile(sig=b"acsx"), it.expect_err(C.ERR_HDR), "no acsp"))
    recs.append(Rec(it.OP_ICCCHK, 0, icc_profile(version=1), it.expect_err(C.ERR_UNSUPPORTED), "v1"))
    recs.append(Rec(it.OP_ICCCHK, 0, icc_profile(version=6), it.expect_err(C.ERR_UNSUPPORTED), "v6"))
    recs.append(Rec(it.OP_ICCCHK, 0, icc_profile(bad_tag=True), it.expect_err(C.ERR_HDR), "tag outside"))
    p = bytearray(prof)
    struct.pack_into(">I", p, 128, 100000)
    recs.append(Rec(it.OP_ICCCHK, 0, bytes(p), it.expect_err(C.ERR_HDR), "tag count"))
    p = bytearray(prof)
    struct.pack_into(">I", p, 0, 100)
    recs.append(Rec(it.OP_ICCCHK, 0, bytes(p), it.expect_err(C.ERR_HDR), "declared size < 132"))
    # segments
    for n in (0, 1, 1000, 65519, 65520, 200000, 255 * 65519):
        data = bytes((k * 13 + 5) & 0xFF for k in range(n))
        exp = b"".join(icc_segments_py(data)) if n else b""
        recs.append(Rec(it.OP_ICCSEG, 0, data, it.expect_bytes(exp), f"segments {n}"))
    recs.append(Rec(it.OP_ICCSEG, 0, b"\0" * (255 * 65519 + 1), it.expect_err(C.ERR_LIMIT), "256 segments"))
    # collect
    base = encode_file(runner, C.IMGF_JPEG, smooth_bgra(rng, 24, 16), C.FMT_PARGB, quality=80)
    big = bytes((k * 29 + 1) & 0xFF for k in range(150000))

    def col_ok(profile):
        def chk(res, out):
            if res != 1:
                return f"result {it.err_name(res)}, expected 1"
            return None if bytes(out) == profile else f"profile differs ({len(out)} vs {len(profile)})"
        return chk
    for prof_, nm in ((prof, "small"), (big, "3 segments")):
        segs = icc_segments_py(prof_)
        recs.append(Rec(it.OP_ICCCOL, 0, it.jpeg_insert(base, segs), col_ok(prof_), f"collect {nm}"))
        sh = segs[:]
        rng.shuffle(sh)
        other = it.jpeg_seg(0xE2, b"FPXR\0" + b"\1" * 40)
        recs.append(Rec(it.OP_ICCCOL, 0, it.jpeg_insert(base, [other] + sh), col_ok(prof_), f"collect shuffled {nm}"))
    recs.append(Rec(it.OP_ICCCOL, 0, base, it.expect_err(0), "no profile"))
    segs = icc_segments_py(big)

    def seg_edit(i, seq=None, cnt=None):
        s = bytearray(segs[i])
        if seq is not None:
            s[16] = seq
        if cnt is not None:
            s[17] = cnt
        return bytes(s)
    bad_sets = {
        "count mismatch": [segs[0], seg_edit(1, cnt=4), segs[2]],
        "seq 0": [segs[0], seg_edit(1, seq=0), segs[2]],
        "seq > count": [segs[0], seg_edit(1, seq=4), segs[2]],
        "duplicate": [segs[0], segs[1], segs[1], segs[2]],
        "missing": [segs[0], segs[2]],
        "count 0": [seg_edit(0, seq=1, cnt=0)],
    }
    for nm, ss in bad_sets.items():
        recs.append(Rec(it.OP_ICCCOL, 0, it.jpeg_insert(base, ss), it.expect_err(C.ERR_CHUNK), f"collect {nm}"))
    empty_seg = it.jpeg_seg(0xE2, b"ICC_PROFILE\0\x01\x01")
    recs.append(Rec(it.OP_ICCCOL, 0, it.jpeg_insert(base, [empty_seg]), it.expect_err(C.ERR_CHUNK), "collect empty"))
    # after SOS: not seen
    sos = base.index(b"\xff\xda")
    after = base[:-2] + b"".join(icc_segments_py(prof)) + base[-2:]
    recs.append(Rec(it.OP_ICCCOL, 0, after, it.expect_err(0), "segments after SOS"))
    assert sos > 0
    # IMGD_ICC through the decoders
    jp = it.jpeg_insert(base, icc_segments_py(big))

    def icc_meta(profile):
        def chk(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            return None if d["icc"] == profile else f"icc {len(d['icc'])} bytes, expected {len(profile)}"
        return chk
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(jp, C.IMGD_ICC), icc_meta(big), "JPEG IMGD_ICC"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(jp, 0), icc_meta(b""), "JPEG without IMGD_ICC"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(jp, C.IMGD_ICC, usemeta=0), icc_meta(b""), "JPEG no IMGMETA"))
    px = it.rand_bgra(rng, 9, 7, "mixed")
    iccp = it.png_chunk(b"iCCP", b"jfgui test\0\0" + zlib.compress(big, 9))
    png = it.png_make(px[..., [2, 1, 0, 3]], extra_chunks=[iccp])
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, C.IMGD_ICC), icc_meta(big), "PNG iCCP"))
    bmp = bmp_v5(px, icc=prof)
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(bmp, C.IMGD_ICC),
                    it.expect_decex(it.premul(px), C.FMT_PARGB), "BMP V5 pixels"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(bmp, C.IMGD_ICC), icc_meta(prof), "BMP V5 profile"))
    return recs


# ================================================================== detect

def suite_detect(rng, core_only):
    recs = []

    def det(data, exp, name, hint=0):
        recs.append(Rec(it.OP_PROBE, 256 | hint, data, it.expect_err(exp), name))
    F = C
    det(b"\x89PNG\r\n\x1a\n" + b"\0" * 8, F.IMGF_PNG, "png")
    det(b"\xff\xd8\xff\xe0", F.IMGF_JPEG, "jpeg")
    det(b"\xff\xd8\xfe", 0, "jpeg bad third byte")
    det(b"GIF87a", F.IMGF_GIF, "gif87a")
    det(b"GIF89a\0\0", F.IMGF_GIF, "gif89a")
    det(b"GIF88a\0\0", 0, "gif88a")
    det(b"DDS \x7c\0\0\0", F.IMGF_DDS, "dds")
    det(b"RIFF\x10\0\0\0WEBPVP8 ", F.IMGF_WEBP, "webp")
    det(b"RIFF\x10\0\0\0WAVEfmt ", 0, "wav")
    det(b"II*\0\x08\0\0\0", F.IMGF_TIFF, "tiff II")
    det(b"MM\0*\0\0\0\x08", F.IMGF_TIFF, "tiff MM")
    det(b"II+\0\x08\0\0\0\x10\0\0\0\0\0\0\0", F.IMGF_TIFF, "bigtiff II")
    det(b"MM\0+\0\x08\0\0", F.IMGF_TIFF, "bigtiff MM")
    det(b"II\xbc\x01\x08\0\0\0", F.IMGF_JXR, "jxr")
    det(b"II\xbc\x00\x08\0\0\0", F.IMGF_JXR, "jxr v0")
    det(b"\xff\x0a\xfa\x7f", F.IMGF_JXL, "jxl codestream")
    det(b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl ", F.IMGF_JXL, "jxl container")
    det(b"PDN3\x10\0\0<pdnImage", F.IMGF_PDN, "pdn")
    det(gzip.compress(b"legacy pdn", mtime=0), F.IMGF_PDN, "gzip -> legacy pdn")
    for hs in (12, 40, 52, 56, 64, 108, 124):
        det(b"BM" + b"\0" * 12 + struct.pack("<I", hs) + b"\0" * 8, F.IMGF_BMP, f"bmp header {hs}")
    for hs in (0, 16, 41, 128):
        det(b"BM" + b"\0" * 12 + struct.pack("<I", hs) + b"\0" * 8, 0, f"bmp bad header {hs}")
    det(b"BM" + b"\0" * 10, F.IMGF_BMP, "bmp short (truncated)")
    # ISOBMFF
    av = [b"mif1", b"avif", b"miaf"]
    he = [b"mif1", b"heic"]
    det(ftyp_box(b"avif", av), F.IMGF_AVIF, "ftyp avif")
    det(ftyp_box(b"avis", [b"avis", b"msf1"]), F.IMGF_AVIF, "ftyp avis")
    det(ftyp_box(b"heic", he), F.IMGF_HEIC, "ftyp heic")
    det(ftyp_box(b"heix", he), F.IMGF_HEIC, "ftyp heix")
    det(ftyp_box(b"mif1", [b"mif1", b"avif"]), F.IMGF_AVIF, "mif1 + avif")
    det(ftyp_box(b"mif1", [b"mif1", b"heic"]), F.IMGF_HEIC, "mif1 + heic")
    for prim, exp in ((1, F.IMGF_HEIC), (2, F.IMGF_AVIF)):
        items = [(1, b"hvc1"), (2, b"av01"), (3, b"Exif")]
        for pv, iv in ((0, 2), (1, 3)):
            f = ftyp_box(b"mif1", [b"mif1", b"miaf"]) + heif_meta(prim, items, pv, iv)
            det(f, exp, f"mif1 primary {prim} pitm v{pv} infe v{iv}")
            f = ftyp_box(b"mif1", [b"mif1", b"avif", b"heic"]) + heif_meta(prim, items, pv, iv)
            det(f, exp, f"avif+heic brands primary {prim} v{pv}")
    f = ftyp_box(b"mif1", [b"mif1"]) + it.bmff_box(b"free", b"\0" * 20) + heif_meta(7, [(7, b"grid"), (8, b"av01")])
    det(f, F.IMGF_AVIF, "grid primary, first coded item av01")
    det(ftyp_box(b"mif1", [b"mif1"]), 0, "mif1 without items")
    # boxes without payload exactly at the end of the data (guard page right behind them)
    empty_infe = it.bmff_box(b"infe")
    iinf = it.bmff_fullbox(b"iinf", 0, 0, struct.pack(">H", 2) +
                           it.bmff_fullbox(b"infe", 2, 0, struct.pack(">HH", 1, 0) + b"av01\0") + empty_infe)
    det(ftyp_box(b"mif1", [b"mif1"]) + it.bmff_fullbox(b"meta", 0, 0, iinf), F.IMGF_AVIF, "empty infe at the end")
    det(ftyp_box(b"mif1", [b"mif1"]) + it.bmff_fullbox(b"meta", 0, 0, it.bmff_box(b"pitm")), 0, "empty pitm at the end")
    det(ftyp_box(b"mif1", [b"mif1"]) + it.bmff_fullbox(b"meta", 0, 0, it.bmff_box(b"iinf")), 0, "empty iinf at the end")
    det(ftyp_box(b"mif1", [b"mif1"]) + it.bmff_box(b"meta"), 0, "empty meta at the end")
    det(ftyp_box(b"isom", [b"isom", b"mp41"]), 0, "mp4")
    det(ftyp_box(b"crx ", [b"crx "]), 0, "cr3")
    f = ftyp_box(b"mif1", [b"mif1"]) + it.bmff_box(b"meta", b"\0" * 4 + it.bmff_box(b"pitm", b"\0" * 100), large=True)
    det(f, 0, "broken meta")
    # TGA: hint, heuristic
    good = tga_header(4, 3) + bytes(4 * 3 * 4)
    rle = tga_header(4, 3, typ=10) + b"\x8b" + b"\x10\x20\x30\x40"
    mapped = tga_header(4, 2, typ=1, depth=8, desc=0, cmap=(1, 0, 4, 24)) + bytes(12) + bytes(8)
    grey = tga_header(5, 2, typ=3, depth=8, desc=0) + bytes(10)
    for data, nm in ((good, "tga 32"), (rle, "tga rle"), (mapped, "tga mapped"), (grey, "tga grey")):
        det(data, 0 if core_only else F.IMGF_TGA, f"{nm} heuristic")
        det(data, F.IMGF_TGA, f"{nm} hint", hint=F.IMGF_TGA)
    bad_tgas = {
        "cmap type 2": tga_header(4, 3, cmap=(2, 0, 0, 0)) + bytes(48),
        "type 4": tga_header(4, 3, typ=4) + bytes(48),
        "w 0": tga_header(0, 3) + bytes(48),
        "depth 12": tga_header(4, 3, depth=12, desc=0) + bytes(48),
        "desc bits 6": tga_header(4, 3, desc=0x48) + bytes(48),
        "alpha 4 of 32": tga_header(4, 3, desc=4) + bytes(48),
        "missing pixels": tga_header(4, 3) + bytes(47),
        "mapped without map": tga_header(4, 3, typ=1, depth=8, desc=0) + bytes(12),
        "stray map fields": tga_header(4, 3, cmap=(0, 0, 0, 24)) + bytes(48),
        "no pixel data": tga_header(4, 3, typ=10),
    }
    for nm, data in bad_tgas.items():
        det(data, 0, f"not tga: {nm}")
    det(b"random bytes that are no image at all", 0, "garbage")
    det(b"random bytes", F.IMGF_TGA, "garbage with TGA hint", hint=F.IMGF_TGA)
    det(b"", 0, "empty")
    det(b"\x89PNG\r\n\x1a\n", F.IMGF_PNG, "png with TGA hint", hint=F.IMGF_TGA)
    # probe of a registered TGA fills the meta record
    if not core_only:
        def chk(res, out):
            if res != F.IMGF_TGA:
                return f"result {res}"
            m = it.parse_meta_rec(out)
            want = dict(w=4, h=3, bits=8, model=F.IMM_RGB, channels=4, frames=1)
            got = {k: m[k] for k in want}
            if got != want or not m["flags"] & F.IMF_ALPHA:
                return f"meta {got} flags {m['flags']:#x}"
            return None
        recs.append(Rec(it.OP_PROBE, 0, good, chk, "tga probe meta"))
    return recs


# ================================================================== probe

def meta_expect(exp_fmt, **want):
    flags_all = want.pop("flags_all", 0)
    flags_none = want.pop("flags_none", 0)

    def chk(res, out):
        if res != exp_fmt:
            return f"result {res}, expected {exp_fmt}"
        m = it.parse_meta_rec(out)
        for k, v in want.items():
            if m[k] != v:
                return f"{k} = {m[k]}, expected {v} (meta {m})"
        if m["flags"] & flags_all != flags_all:
            return f"flags {m['flags']:#x} lack {flags_all:#x}"
        if m["flags"] & flags_none:
            return f"flags {m['flags']:#x} contain {flags_none & m['flags']:#x}"
        return None
    return chk


def suite_probe(rng, runner):
    recs = []
    F = C
    rgba = it.rand_bgra(rng, 5, 3, "mixed")[..., [2, 1, 0, 3]]
    chunks = [it.png_chunk(b"pHYs", struct.pack(">IIB", 3780, 7560, 1)),
              it.png_chunk(b"gAMA", struct.pack(">I", 45455)),
              it.png_chunk(b"sRGB", b"\0"),
              it.png_chunk(b"cICP", bytes([1, 13, 0, 1])),
              it.png_chunk(b"acTL", struct.pack(">II", 3, 0)),
              it.png_chunk(b"fcTL", struct.pack(">IIIIIHHBB", 0, 5, 3, 0, 0, 1, 10, 0, 0))]
    png = it.png_make(rgba, extra_chunks=chunks)
    recs.append(Rec(it.OP_PROBE, 0, png, meta_expect(
        F.IMGF_PNG, w=5, h=3, bits=8, model=F.IMM_RGB, channels=4, frames=3, dpix=dpi16_ppm(3780),
        dpiy=dpi16_ppm(7560), gamma=45455, cicp=0x01000D01, delay=100, loops=0, resunit=F.IMR_CM,
        orient=1, flags_all=F.IMF_ALPHA | F.IMF_LOSSLESS | F.IMF_ANIMATED | F.IMF_SRGB), "png chunks"))
    # 16-bit grey interlaced (header only matters)
    ihdr = struct.pack(">IIBBBBB", 300, 200, 16, 0, 0, 0, 1)
    png16 = b"\x89PNG\r\n\x1a\n" + it.png_chunk(b"IHDR", ihdr) + it.png_chunk(b"IEND", b"")
    recs.append(Rec(it.OP_PROBE, 0, png16, meta_expect(
        F.IMGF_PNG, w=300, h=200, bits=16, model=F.IMM_GRAY, channels=1,
        flags_all=F.IMF_HDR | F.IMF_INTERLACED, flags_none=F.IMF_ALPHA), "png 16-bit grey"))
    ihdr = struct.pack(">IIBBBBB", 8, 8, 4, 3, 0, 0, 0)
    pal = b"\x89PNG\r\n\x1a\n" + it.png_chunk(b"IHDR", ihdr) + it.png_chunk(b"PLTE", b"\0" * 48) + \
        it.png_chunk(b"tRNS", b"\0\x80") + it.png_chunk(b"IEND", b"")
    recs.append(Rec(it.OP_PROBE, 0, pal, meta_expect(F.IMGF_PNG, bits=4, model=F.IMM_PAL, channels=1,
                                                     flags_all=F.IMF_ALPHA), "png palette tRNS"))
    recs.append(Rec(it.OP_PROBE, 0, png[:20], meta_expect(F.IMGF_PNG, w=0), "png truncated"))
    recs.append(Rec(it.OP_PROBE, 512, png, it.expect_err(F.IMGF_PNG), "png, no meta"))
    # JPEG
    base = encode_file(runner, F.IMGF_JPEG, smooth_bgra(rng, 21, 13), F.FMT_PARGB, quality=85)
    recs.append(Rec(it.OP_PROBE, 0, base, meta_expect(
        F.IMGF_JPEG, w=21, h=13, bits=8, model=F.IMM_YCC, channels=3, dpix=96 << 16, dpiy=96 << 16,
        resunit=F.IMR_INCH, frames=1, orient=1, flags_none=F.IMF_INTERLACED | F.IMF_LOSSLESS), "jpeg jfif"))
    ex = it.jpeg_seg(0xE1, b"Exif\0\0" + it.exif_blob(orient=6, xres=(300, 1), yres=(150, 1), unit=2, colorspace=1))
    nojfif = jpeg_strip_app0(base)
    recs.append(Rec(it.OP_PROBE, 0, it.jpeg_insert(nojfif, [ex], after_app0=False), meta_expect(
        F.IMGF_JPEG, w=21, h=13, orient=6, dpix=300 << 16, dpiy=150 << 16, resunit=F.IMR_INCH,
        flags_all=F.IMF_SRGB), "jpeg exif dpi"))
    recs.append(Rec(it.OP_PROBE, 0, it.jpeg_insert(base, [ex]), meta_expect(
        F.IMGF_JPEG, orient=6, dpix=96 << 16), "jpeg jfif wins"))
    ex_cm = it.jpeg_seg(0xE1, b"Exif\0\0" + it.exif_blob(orient=1, xres=(100, 1), yres=(100, 1), unit=3))
    recs.append(Rec(it.OP_PROBE, 0, it.jpeg_insert(nojfif, [ex_cm], after_app0=False), meta_expect(
        F.IMGF_JPEG, dpix=exif_dpi_ref(100, 1, 3), resunit=F.IMR_CM), "jpeg exif dpcm"))
    # JFIF in dots per cm
    j = bytearray(base)
    j[13] = 2
    struct.pack_into(">HH", j, 14, 40, 40)
    recs.append(Rec(it.OP_PROBE, 0, bytes(j), meta_expect(
        F.IMGF_JPEG, dpix=(40 * 65536 * 254 + 50) // 100, resunit=F.IMR_CM), "jpeg jfif dpcm"))
    prog = bytearray(base)
    i = prog.index(b"\xff\xc0")
    prog[i + 1] = 0xC2
    recs.append(Rec(it.OP_PROBE, 0, bytes(prog), meta_expect(F.IMGF_JPEG, flags_all=F.IMF_INTERLACED),
                    "jpeg progressive flag"))
    adobe = it.jpeg_seg(0xEE, b"Adobe\0\x64\0\0\0\0\0")
    recs.append(Rec(it.OP_PROBE, 0, it.jpeg_insert(base, [adobe]), meta_expect(F.IMGF_JPEG, model=F.IMM_RGB),
                    "jpeg adobe transform 0"))
    grey = encode_file(runner, F.IMGF_JPEG, np.full((6, 9), 77, np.uint8), F.FMT_A8, quality=90)
    recs.append(Rec(it.OP_PROBE, 0, grey, meta_expect(F.IMGF_JPEG, w=9, h=6, model=F.IMM_GRAY, channels=1),
                    "jpeg grey"))
    recs.append(Rec(it.OP_PROBE, 0, base[:30], meta_expect(F.IMGF_JPEG, w=0), "jpeg truncated"))
    # GIF
    g = gif_anim([[0, 1, 2, 3, 0, 1], [3, 2, 1, 0, 3, 2]], 3, 2, loops=3, delay=20)
    recs.append(Rec(it.OP_PROBE, 0, g, meta_expect(
        F.IMGF_GIF, w=3, h=2, frames=2, loops=3, delay=200, bits=2, model=F.IMM_PAL, channels=1,
        flags_all=F.IMF_ANIMATED | F.IMF_ALPHA), "gif animation"))
    g1 = gif_anim([[0, 1, 2, 3]], 2, 2, loops=0, delay=5, transp=False)
    recs.append(Rec(it.OP_PROBE, 0, g1, meta_expect(F.IMGF_GIF, frames=1, delay=50,
                                                    flags_none=F.IMF_ANIMATED | F.IMF_ALPHA), "gif still"))
    recs.append(Rec(it.OP_PROBE, 0, g[:40], meta_expect(F.IMGF_GIF, w=3, h=2), "gif truncated"))
    # BMP
    op = encode_file(runner, F.IMGF_BMP, it.rand_bgra(rng, 7, 4, "opaque"), F.FMT_PARGB,
                     bitdepth=24)                   # auto would pick an exact 8-bit palette (CA)
    recs.append(Rec(it.OP_PROBE, 0, op, meta_expect(F.IMGF_BMP, w=7, h=4, bits=8, model=F.IMM_RGB, channels=3,
                                                    flags_none=F.IMF_ALPHA), "bmp 24"))
    al = encode_file(runner, F.IMGF_BMP, it.premul(it.rand_bgra(rng, 7, 4, "mixed")), F.FMT_PARGB)
    recs.append(Rec(it.OP_PROBE, 0, al, meta_expect(F.IMGF_BMP, w=7, h=4, channels=4, flags_all=F.IMF_ALPHA),
                    "bmp 32 alpha"))
    v5 = bmp_v5(it.rand_bgra(rng, 3, 5, "mixed"), ppm=3937)
    recs.append(Rec(it.OP_PROBE, 0, v5, meta_expect(F.IMGF_BMP, w=3, h=5, dpix=dpi16_ppm(3937), resunit=F.IMR_CM,
                                                    flags_all=F.IMF_ALPHA | F.IMF_SRGB), "bmp v5"))
    return recs


# ================================================================== format table

FMT_TABLE = {
    1: ("PNG", "png=PNG", "image/png"),
    2: ("JPEG", "jpg;jpeg;jpe;jfif;exif=JPEG", "image/jpeg"),
    3: ("BMP", "bmp;dib;rle=BMP", "image/bmp"),
    4: ("GIF", "gif=GIF", "image/gif"),
    5: ("TIFF", "tif;tiff=TIFF", "image/tiff"),
    6: ("TGA", "tga=TGA", "image/x-tga"),
    7: ("DDS", "dds=DDS", "image/vnd-ms.dds"),
    8: ("WebP", "webp=WebP", "image/webp"),
    9: ("AVIF", "avif=AVIF", "image/avif"),
    10: ("HEIC", "heic;heif=HEIC", "image/heic"),
    11: ("JPEG XL", "jxl=JPEG XL", "image/jxl"),
    12: ("JPEG XR", "jxr;wdp;hdp;wmp=JPEG XR", "image/jxr"),
    13: ("Paint.NET", "pdn=Paint.NET", "image/x-PaintCraft"),
}


CAPS = {}      # format -> IFC_* capability bits reported by the binary (load_caps)


def load_caps(runner):
    """Ask the binary which formats decode / encode today, so the expectations follow the
    codecs as they are implemented (stubs report no IFC_DECODE / IFC_ENCODE)."""
    CAPS.clear()
    for f in FMT_TABLE:
        res, out = runner.one(it.OP_FMTINFO, f, b"")
        if res == 0 and len(out) >= 12:
            CAPS[f] = struct.unpack_from("<III", out, len(out) - 12)[0]
        else:
            CAPS[f] = 0


def has_cap(f, bits):
    return CAPS.get(f, 0) & bits == bits


def suite_fmtinfo(rng, core_only):
    recs = []
    F = C
    for f in range(0, 16):
        if f not in FMT_TABLE:
            recs.append(Rec(it.OP_FMTINFO, f, b"", it.expect_err(-1), f"info {f}"))
            continue

        def chk(res, out, f=f):
            if res != 0:
                return f"error {res}"
            parts = bytes(out).split(b"\0")
            names = tuple(p.decode() for p in parts[:3])
            if names != FMT_TABLE[f]:
                return f"strings {names}"
            caps, iom, reg = struct.unpack_from("<III", out, len(out) - 12)
            core = f <= 4
            if bool(caps & F.IFC_CORE) != core:
                return f"IFC_CORE {caps:#x}"
            if core and caps & 3 != 3:
                return f"core caps {caps:#x}"
            want = CAPS.get(f, 0) & 3 if (core or not core_only) else 0   # unregistered: no codec
            if caps & 3 != want:
                return f"caps {caps:#x}, expected decode/encode bits {want:#x}"
            if reg != (1 if (core or not core_only) else 0):
                return f"registered {reg}"
            if f in (1, 2, 3, 4) and iom != (F.PNG_IOM, F.JPEG_IOM, F.BMP_IOM, F.GIF_IOM)[f - 1]:
                return f"IOM {iom:#x}"
            if f == 13 and not caps & F.IFC_LAYERS:
                return "PDN without IFC_LAYERS"
            return None
        recs.append(Rec(it.OP_FMTINFO, f, b"", chk, f"info {FMT_TABLE[f][0]}"))
    exts = {}
    for f, (_, e, _) in FMT_TABLE.items():
        for x in e.split("=")[0].split(";"):
            exts[x] = f
    for x, f in exts.items():
        for path in (f"a.{x}", f"C:/Bilder/Übersicht.{x.upper()}", f"/home/u/x.y.{x}", f".{x}"):
            recs.append(Rec(it.OP_FROMPATH, 0, path.encode(), it.expect_err(f), f"path {path}"))
    for path in ("noext", "a.xyz", "a.png.bak", "a.tar.gz", "", "png", "a.jpgx", "a.p"):
        recs.append(Rec(it.OP_FROMPATH, 0, path.encode(), it.expect_err(0), f"path '{path}'"))
    all_list = b"".join(FMT_TABLE[f][1].encode() + b"\0" for f in range(1, 14)) + b"\0"

    def cap_list(need):
        fs = [f for f in range(1, 14) if (f <= 4 or not core_only) and has_cap(f, need)]
        return b"".join(FMT_TABLE[f][1].encode() + b"\0" for f in fs) + b"\0"
    core_list = cap_list(F.IFC_DECODE)
    for need in (F.IFC_DECODE, F.IFC_ENCODE, F.IFC_DECODE | F.IFC_ENCODE):
        exp = cap_list(need)
        recs.append(Rec(it.OP_FILTER, need, b"", lambda res, out, exp=exp: None if res == len(exp) and bytes(out) == exp
                        else f"{res} {bytes(out)!r}", f"filter {need}"))
    recs.append(Rec(it.OP_FILTER, 0, b"", lambda res, out: None if bytes(out) == all_list else f"{bytes(out)!r}",
                    "filter all"))
    lay = FMT_TABLE[13][1].encode() + b"\0\0"
    recs.append(Rec(it.OP_FILTER, F.IFC_LAYERS, b"", lambda res, out: None if bytes(out) == lay else f"{bytes(out)!r}",
                    "filter layers"))
    recs.append(Rec(it.OP_FILTER, F.IFC_DECODE | (10 << 16), b"",
                    lambda res, out: None if res == 0 else f"result {res}", "filter small buffer"))
    recs.append(Rec(it.OP_FILTER, F.IFC_DECODE | (len(core_list) << 16), b"",
                    lambda res, out: None if res == len(core_list) else f"result {res}", "filter exact buffer"))
    return recs


# ================================================================== decode_ex

def suite_decex(rng, runner, core_only):
    recs = []
    F = C
    files = {}
    straight_src = {}                   # file -> straight source pixels (for a native straight path)
    S = it.rand_bgra(rng, 13, 9, "mixed")
    files["png rgba"] = (it.png_make(S[..., [2, 1, 0, 3]]), it.premul(S))
    S2 = it.rand_bgra(rng, 10, 6, "opaque")
    files["png rgb"] = (it.png_make(S2[..., [2, 1, 0]]), S2)
    files["png grey"] = (it.png_make(S2[..., 1]), None)
    files["jpeg"] = (encode_file(runner, F.IMGF_JPEG, smooth_bgra(rng, 33, 17), F.FMT_PARGB, quality=70), None)
    files["bmp24"] = (encode_file(runner, F.IMGF_BMP, S2, F.FMT_PARGB), S2)
    files["bmp32"] = (encode_file(runner, F.IMGF_BMP, it.premul(S), F.FMT_PARGB), it.premul(S))
    files["gif"] = (gif_anim([[0, 1, 2, 3, 0, 1], [3, 3, 3, 3, 3, 3]], 3, 2), None)
    files["bmp v5"] = (bmp_v5(S), it.premul(S))
    straight_src.update({"png rgba": S, "bmp v5": S, "png rgb": S2, "bmp24": S2})
    # classic vs new: same pixels, same error codes (truncations too)
    for name, (data, ref) in files.items():
        res, old = old_decode(runner, data)
        assert res == 0, name
        if ref is not None:
            assert np.array_equal(old, ref), f"{name}: classic decode != reference"
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data), it.expect_decex(old, F.FMT_PARGB), f"{name} = classic"))
        # straight: unpremultiplied from the premultiplied decode (today) or the file's own
        # straight pixels (a native straight path, P1)
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, F.IMGD_STRAIGHT),
                        it.expect_any(it.expect_decex(it.unpremul(old), F.FMT_ARGB),
                                      it.expect_decex(straight_src.get(name, it.unpremul(old)), F.FMT_ARGB)),
                        f"{name} straight"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data, usemeta=0), it.expect_decex(old, F.FMT_PARGB),
                        f"{name} no meta"))
        cuts = sorted(set([0, 1, 2, 5, 9, 17, 30, 60, len(data) // 2, len(data) - 1]))
        for cut in cuts:
            if cut >= len(data):
                continue
            ores = runner.one(it.OP_IMGDEC, 0, data[:cut])[0]
            exp = ores

            def chk(res, out, exp=exp, cut=cut):
                if exp == 0:
                    return None if res == 0 else f"classic ok, new {it.err_name(res)}"
                if res != exp and not (cut < 2 and res == F.ERR_TRUNC) and not (exp == F.ERR_UNSUPPORTED and res in (F.ERR_TRUNC,)):
                    return f"classic {it.err_name(exp)}, new {it.err_name(res)}"
                return None
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data[:cut]), chk, f"{name} cut {cut}"))
    # straight path: alpha 0 -> 0, alpha 255 -> exact source
    def straight_src(res, out):
        d = it.parse_decex(out)
        px = d["px"]
        a = S[..., 3]
        # alpha 0: transparent; the colour is 0 (derived from premultiplied data) or the
        # file's own straight colour (native straight decoders, CA)
        z, sz = px[a == 0], S[a == 0]
        colour_ok = not z.any() or np.array_equal(z, sz)
        if not np.array_equal(px[a == 255], S[a == 255]) or z[:, 3].any() or not colour_ok:
            return "alpha 0 / 255 pixels not exact"
        return None
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(files["png rgba"][0], F.IMGD_STRAIGHT), straight_src, "straight 0/255 exact"))
    png = files["png rgba"][0]
    # flags / limits
    def probe_chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        d = it.parse_decex(out)
        if d["w"] or d["px"] is not None:
            return "pixels returned"
        return None if (d["meta"]["w"], d["meta"]["h"], d["meta"]["fmt"]) == (13, 9, 1) else f"meta {d['meta']}"
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, F.IMGD_PROBE), probe_chk, "IMGD_PROBE"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, maxpx=13 * 9 - 1), it.expect_err(F.ERR_LIMIT), "maxpx - 1"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, maxpx=13 * 9), it.expect_decex(it.premul(S)), "maxpx exact"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(files["jpeg"][0], maxpx=100), it.expect_err(F.ERR_LIMIT), "jpeg maxpx"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, cancel=1), it.expect_err(F.ERR_CANCEL), "cancel"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, frame=1), it.expect_err(F.ERR_UNSUPPORTED), "frame 1"))
    big = b"\x89PNG\r\n\x1a\n" + it.png_chunk(b"IHDR", struct.pack(">IIBBBBB", 20000, 20000, 8, 6, 0, 0, 0))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(big), it.expect_err(F.ERR_LIMIT), "20000^2 -> LIMIT"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(b"not an image"), it.expect_err(F.ERR_UNSUPPORTED), "unknown"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(b"x"), it.expect_err(F.ERR_TRUNC), "1 byte"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(b""), it.expect_err(F.ERR_TRUNC), "empty"))
    for nm, data, fmt in (("tiff", b"II*\0\x08\0\0\0" + b"\0" * 30, F.IMGF_TIFF), ("webp", b"RIFF\x10\0\0\0WEBPVP8L" + b"\0" * 20, F.IMGF_WEBP),
                          ("pdn", b"PDN3" + b"\0" * 30, F.IMGF_PDN), ("dds", b"DDS " + b"\0" * 124, F.IMGF_DDS)):
        if has_cap(fmt, F.IFC_DECODE):
            continue                    # implemented: covered by that codec's own checker
        def chk(res, out, fmt=fmt):
            if res != F.ERR_UNSUPPORTED:
                return f"result {it.err_name(res)}"
            d = it.parse_decex(out)
            return None if d["meta"]["fmt"] == fmt else f"meta fmt {d['meta']['fmt']}"
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(data), chk, f"stub {nm}"))
    # orientation: JPEG
    base = encode_file(runner, F.IMGF_JPEG, smooth_bgra(rng, 11, 7), F.FMT_PARGB, quality=90)
    res, stored = old_decode(runner, base)
    for o in range(1, 9):
        for bo in ("<", ">"):
            j = it.jpeg_insert(base, [it.jpeg_seg(0xE1, b"Exif\0\0" + it.exif_blob(orient=o, bo=bo))])
            res, cl = old_decode(runner, j)
            assert res == 0 and np.array_equal(cl, it.orient(stored, o)), f"classic orientation {o}"
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(j), it.expect_decex(cl, meta=dict(orient=o)),
                            f"jpeg orient {o} {bo} = classic"))
            flags_ok = (lambda res, out, o=o: None if (it.parse_decex(out)["meta"]["flags"] & F.IMF_ORIENT_APPLIED) == (F.IMF_ORIENT_APPLIED if o > 1 else 0)
                        else "ORIENT_APPLIED flag")
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(j), flags_ok, f"jpeg orient {o} {bo} flag"))
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(j, F.IMGD_NO_ORIENT), it.expect_decex(stored, meta=dict(orient=o)),
                            f"jpeg orient {o} {bo} NO_ORIENT"))
    # orientation: PNG eXIf (applied by the dispatcher)
    for o in range(1, 9):
        p = it.png_make(S[..., [2, 1, 0, 3]], extra_chunks=[it.png_chunk(b"eXIf", it.exif_blob(orient=o))])
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(p), it.expect_decex(it.orient(it.premul(S), o), meta=dict(orient=o, w=13, h=9)),
                        f"png eXIf {o}"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(p, F.IMGD_NO_ORIENT | F.IMGD_STRAIGHT),
                        it.expect_any(it.expect_decex(it.unpremul(it.premul(S)), F.FMT_ARGB, meta=dict(orient=o)),
                                      it.expect_decex(S, F.FMT_ARGB, meta=dict(orient=o))), f"png eXIf {o} NO_ORIENT"))
    # EXIF / XMP blobs
    blob = it.exif_blob(orient=1, xres=(300, 1), unit=2, colorspace=1)
    xml = b'<?xpacket begin="\xef\xbb\xbf"?><x:xmpmeta xmlns:x="adobe:ns:meta/">jfgui</x:xmpmeta><?xpacket end="w"?>'
    j = it.jpeg_insert(base, [it.jpeg_seg(0xE1, b"Exif\0\0" + blob), xmp_app1(xml)])

    def blobs(exif=None, xmp=None):
        def chk(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            if exif is not None and d["exif"] != exif:
                return f"exif {len(d['exif'])} bytes"
            if xmp is not None and d["xmp"] != xmp:
                return f"xmp {d['xmp'][:40]!r}"
            return None
        return chk
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(j, F.IMGD_EXIF | F.IMGD_XMP), blobs(blob, xml), "jpeg exif+xmp"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(j, F.IMGD_PROBE | F.IMGD_EXIF), blobs(blob, b""), "jpeg probe exif"))
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(j), blobs(b"", b""), "jpeg no blob flags"))
    itxt = b"XML:com.adobe.xmp\0\0\0\0\0" + xml
    itxt_z = b"XML:com.adobe.xmp\0\1\0en\0\0" + zlib.compress(xml)
    for nm, ch in (("plain", itxt), ("compressed", itxt_z)):
        p = it.png_make(S[..., [2, 1, 0, 3]], extra_chunks=[it.png_chunk(b"iTXt", ch)],
                        after_idat=[it.png_chunk(b"eXIf", blob)])
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(p, F.IMGD_EXIF | F.IMGD_XMP), blobs(blob, xml), f"png iTXt {nm} + eXIf"))
    # hint: a TGA hint does not override magic
    recs.append(Rec(it.OP_DECEX, 0, it.decex_input(png, hint=F.IMGF_TGA), it.expect_decex(it.premul(S)), "png with TGA hint"))
    return recs


# ================================================================== pixels

def bitread_ref(stream, widths, lsb):
    """Expected mo_bits output: the reads (PEEK 13 does not consume before its SKIP), zero bits
    past the end, then the overrun flag."""
    total = len(stream) * 8
    if lsb:
        val = int.from_bytes(stream, "little")
    else:
        val = int.from_bytes(stream, "big")
    out, pos = [], 0
    for w in widths:
        if lsb:
            v = (val >> pos) & ((1 << w) - 1) if pos < total else 0
        else:
            # bits pos .. pos + w - 1 counted from the most significant bit, zeros past the end
            v = 0
            for k in range(w):
                b = pos + k
                bit = (val >> (total - 1 - b)) & 1 if b < total else 0
                v = (v << 1) | bit
        out.append(v)
        pos += w
    out.append(1 if pos > total else 0)
    return struct.pack(f"<{len(out)}Q", *out)


def suite_pixels(rng):
    recs = []
    F = C
    c, a = np.meshgrid(np.arange(256), np.arange(256))
    straight = np.stack([c, 255 - c, (c * 7) & 255, a], -1).astype(np.uint8)         # 256 x 256
    valid_p = it.premul(straight)
    anyp = np.stack([c, (c * 3) & 255, 255 - c, a], -1).astype(np.uint8)             # c > a too
    for src, sfmt, dfmt, exp, nm in (
            (straight, F.FMT_ARGB, F.FMT_PARGB, valid_p, "ARGB->PARGB"),
            (valid_p, F.FMT_PARGB, F.FMT_ARGB, it.unpremul(valid_p), "PARGB->ARGB"),
            (anyp, F.FMT_PARGB, F.FMT_ARGB, it.unpremul(anyp), "PARGB(c>a)->ARGB")):
        def chk(res, out, exp=exp, dfmt=dfmt):
            if res != 0:
                return f"error {it.err_name(res)}"
            w, h, fmt, fl, px = it.parse_surface(out)
            if fmt != dfmt:
                return f"fmt {fmt}"
            m = it.max_abs(px, exp)
            if m:
                y, x = np.argwhere((px != exp).any(-1))[0]
                return f"max |d| {m} at c={x} a={y}: {list(px[y, x])} vs {list(exp[y, x])}"
            return None
        data = it.surface_input(src, sfmt)
        recs.append(Rec(it.OP_CONVERT, dfmt, data, chk, f"convert {nm}"))
        recs.append(Rec(it.OP_CONVERT, dfmt | 256, data, chk, f"surf_convert {nm}"))

        def rows(res, out, exp=exp):
            px = np.frombuffer(out, np.uint8).reshape(256, 256, 4)
            return None if np.array_equal(px, exp) else f"max |d| {it.max_abs(px, exp)}"
        recs.append(Rec(it.OP_GETROW, dfmt, data, rows, f"get_row {nm}"))
    x = it.rand_bgra(rng, 37, 11, "random")
    xo = x.copy()
    xo[..., 3] = 255
    for dfmt in (F.FMT_PARGB, F.FMT_ARGB):
        recs.append(Rec(it.OP_CONVERT, dfmt, it.surface_input(x, F.FMT_XRGB),
                        lambda res, out: None if res == 0 and np.array_equal(it.parse_surface(out)[4], xo) else "XRGB", f"XRGB->{dfmt}"))
        recs.append(Rec(it.OP_GETROW, dfmt, it.surface_input(x, F.FMT_XRGB),
                        lambda res, out: None if np.array_equal(np.frombuffer(out, np.uint8).reshape(11, 37, 4), xo) else "XRGB rows",
                        f"get_row XRGB {dfmt}"))
        g = x[..., 0]
        gx = np.stack([g, g, g, np.full_like(g, 255)], -1)
        recs.append(Rec(it.OP_CONVERT, dfmt | 256, it.surface_input(g, F.FMT_A8),
                        lambda res, out: None if res == 0 and np.array_equal(it.parse_surface(out)[4], gx) else "A8", f"A8 surf_convert {dfmt}"))
        recs.append(Rec(it.OP_CONVERT, dfmt, it.surface_input(g, F.FMT_A8), it.expect_err(F.ERR_UNSUPPORTED), f"A8 in place {dfmt}"))
        same = x if dfmt == F.FMT_ARGB else it.premul(x)
        recs.append(Rec(it.OP_CONVERT, dfmt, it.surface_input(same, dfmt),
                        lambda res, out, same=same: None if np.array_equal(it.parse_surface(out)[4], same) else "changed", f"same fmt {dfmt}"))
    recs.append(Rec(it.OP_CONVERT, F.FMT_A8, it.surface_input(x, F.FMT_ARGB), it.expect_err(F.ERR_UNSUPPORTED), "to A8"))
    # row converters
    v = np.arange(65536, dtype=np.uint16)
    exp16 = ((v.astype(np.int64) * 2 + 257) // 514).astype(np.uint8)
    recs.append(Rec(it.OP_ROWCONV, 0, v.astype("<u2").tobytes(), it.expect_bytes(exp16.tobytes()), "u16 LE -> u8 (all)"))
    recs.append(Rec(it.OP_ROWCONV, 1, v.astype(">u2").tobytes(), it.expect_bytes(exp16.tobytes()), "u16 BE -> u8 (all)"))
    recs.append(Rec(it.OP_ROWCONV, 5, v.astype("<u2").tobytes(), it.expect_bytes(exp16.tobytes()), "img_u16_to_u8 (all)"))
    for n in (1, 7, 8, 9, 15):
        recs.append(Rec(it.OP_ROWCONV, 1, v[:n].astype(">u2").tobytes(), it.expect_bytes(exp16[:n].tobytes()), f"u16 BE tail {n}"))
    halves = v.view(np.float16)
    f32 = halves.astype(np.float32)

    def f2u8(x):
        x = np.asarray(x, np.float32)
        x = np.where(np.isnan(x), np.float32(0), x)
        x = np.minimum(np.maximum(x, np.float32(0)), np.float32(1))
        return ((x * np.float32(255)) + np.float32(0.5)).astype(np.float32).astype(np.int64).astype(np.uint8)
    recs.append(Rec(it.OP_ROWCONV, 2, v.astype("<u2").tobytes(), it.expect_bytes(f2u8(f32).tobytes()), "half -> u8 (all)"))

    def half_bits(res, out):
        got = np.frombuffer(out, np.uint32)
        ref = f32.view(np.uint32)
        nan = np.isnan(f32)
        if not np.array_equal(got[~nan], ref[~nan]):
            i = np.argwhere(got[~nan] != ref[~nan])[0][0]
            return f"half {v[~nan][i]:#06x}: {got[~nan][i]:#010x} vs {ref[~nan][i]:#010x}"
        g = got[nan].view(np.float32)
        return None if np.isnan(g).all() else "NaN not preserved"
    recs.append(Rec(it.OP_ROWCONV, 6, v.astype("<u2").tobytes(), half_bits, "img_half_to_f32 (all)"))
    r = np.random.default_rng(rng.randrange(1 << 30))
    fl = np.concatenate([r.uniform(-0.5, 1.5, 4000).astype(np.float32),
                         np.array([np.nan, np.inf, -np.inf, -0.0, 0.0, 1.0, np.nextafter(np.float32(1), np.float32(2)),
                                   0.5 / 255, 1.5 / 255, 254.5 / 255, 1e-30, -1e-30], np.float32)])
    for n in (len(fl), 1, 2, 3, 5):
        recs.append(Rec(it.OP_ROWCONV, 3, fl[:n].tobytes(), it.expect_bytes(f2u8(fl[:n]).tobytes()), f"f32 -> u8 {n}"))

    def lin_srgb(i):
        vv = i / 4095
        s = vv * 12.92 if vv <= 0.0031308 else 1.055 * vv ** (1 / 2.4) - 0.055
        return min(255, int(255 * s + 0.5))
    lut = np.array([lin_srgb(i) for i in range(4096)], np.uint8)
    xx = np.where(np.isnan(fl), np.float32(0), fl)
    xx = np.minimum(np.maximum(xx, np.float32(0)), np.float32(1))
    idx = ((xx * np.float32(4095)) + np.float32(0.5)).astype(np.float32).astype(np.int64) & 4095
    recs.append(Rec(it.OP_ROWCONV, 4, fl.tobytes(), it.expect_bytes(lut[idx].tobytes()), "f32 -> sRGB"))
    # img_int.inc bit readers (MSB-first kind 7, LSB-first kind 8)
    for lsb in (0, 1):
        for n_bytes in (0, 1, 7, 8, 9, 15, 16, 17, 40, 300):
            stream = bytes(rng.randrange(256) for _ in range(n_bytes))
            widths = [rng.randrange(1, 57) for _ in range(rng.randrange(0, 40))]
            recs.append(Rec(it.OP_ROWCONV, 7 + lsb, struct.pack("<I", len(widths)) + bytes(widths) + stream,
                            it.expect_bytes(bitread_ref(stream, [5, 13, 56] + widths, lsb)),
                            f"bit reader {'LSB' if lsb else 'MSB'} {n_bytes} bytes {len(widths)} reads"))
    # orientation
    for (w, h) in ((1, 1), (3, 5), (17, 9), (64, 2)):
        src = it.rand_bgra(rng, w, h, "random")
        for fmt in (F.FMT_ARGB, F.FMT_PARGB):
            for o in range(0, 10):
                exp = it.orient(src, o) if 1 <= o <= 8 else src

                def chk(res, out, exp=exp, fmt=fmt):
                    if res != 0:
                        return f"error {it.err_name(res)}"
                    ww, hh, f, fl_, px = it.parse_surface(out)
                    if f != fmt:
                        return f"fmt {f}"
                    return None if px.shape == exp.shape and np.array_equal(px, exp) else f"pixels differ {px.shape} {exp.shape}"
                recs.append(Rec(it.OP_ORIENT, o, it.surface_input(src, fmt), chk, f"orient {o} {w}x{h} fmt {fmt}"))
    return recs


# ================================================================== encode

def suite_encode(rng, runner, core_only):
    recs = []
    F = C
    S = it.rand_bgra(rng, 23, 14, "mixed")
    P = it.premul(S)
    O = it.rand_bgra(rng, 23, 14, "opaque")

    def decodes_to(exp, fmt_flags=0, tol=0, psnr_min=None):
        def chk(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            r2, o2 = runner.one(it.OP_DECEX, 0, it.decex_input(bytes(out), fmt_flags))
            if r2 != 0:
                return f"re-decode {it.err_name(r2)}"
            d = it.parse_decex(o2)
            if psnr_min is not None:
                p = it.psnr(d["px"][..., :3], exp[..., :3])
                return None if p >= psnr_min else f"PSNR {p:.1f}"
            m = it.max_abs(d["px"], exp)
            return None if m <= tol else f"max |d| {m}"
        return chk
    # PNG / BMP from straight and premultiplied input: decode (PARGB) == premul(S) exactly
    for fmt, nm in ((F.IMGF_PNG, "png"), (F.IMGF_BMP, "bmp")):
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(fmt) + it.surface_input(S, F.FMT_ARGB), decodes_to(P), f"{nm} from ARGB"))
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(fmt) + it.surface_input(P, F.FMT_PARGB), decodes_to(P), f"{nm} from PARGB"))
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(fmt) + it.surface_input(O, F.FMT_XRGB), decodes_to(O), f"{nm} from XRGB"))
    for lv in (1, 9, 12):
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_PNG, effort=lv) + it.surface_input(S, F.FMT_ARGB), decodes_to(P), f"png effort {lv}"))
    g = S[..., 1].copy()
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_PNG) + it.surface_input(g, F.FMT_A8),
                    decodes_to(np.stack([g, g, g, np.full_like(g, 255)], -1)), "png from A8"))
    sm = smooth_bgra(rng, 40, 24)
    for q in (0, 30, 95, 100, 150):
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_JPEG, quality=q) + it.surface_input(sm, F.FMT_PARGB),
                        decodes_to(sm, psnr_min=24 if q == 30 else 34), f"jpeg q{q}"))
    sizes = {}

    def size_of(q):
        def chk(res, out):
            sizes[q] = len(out)
            return None if res == 0 else f"error {res}"
        return chk
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_JPEG, quality=20) + it.surface_input(sm, F.FMT_PARGB), size_of(20), "jpeg q20 size"))
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_JPEG, quality=98) + it.surface_input(sm, F.FMT_PARGB), size_of(98), "jpeg q98 size"))
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_JPEG) + it.surface_input(S, F.FMT_ARGB),
                    decodes_to(np.concatenate([(P[..., :3].astype(int) + 255 - P[..., 3:].astype(int)).clip(0, 255).astype(np.uint8),
                                               np.full(P.shape[:2] + (1,), 255, np.uint8)], -1), psnr_min=30), "jpeg ARGB over white"))
    pal = np.array([[0, 0, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255], [9, 9, 9, 255]], np.uint8)
    idx = np.random.default_rng(5).integers(0, 4, (9, 15))
    gp = pal[idx]
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_GIF) + it.surface_input(gp, F.FMT_ARGB), decodes_to(gp), "gif exact"))
    # errors
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(0) + it.surface_input(P, F.FMT_PARGB), it.expect_err(F.ERR_UNSUPPORTED), "fmt 0"))
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(99) + it.surface_input(P, F.FMT_PARGB), it.expect_err(F.ERR_UNSUPPORTED), "fmt 99"))
    for f in range(5, 14):
        if has_cap(f, F.IFC_ENCODE):
            continue                    # implemented: covered by that codec's own checker
        recs.append(Rec(it.OP_ENCODE, 0, imgopt(f) + it.surface_input(P, F.FMT_PARGB), it.expect_err(F.ERR_UNSUPPORTED),
                        f"stub encoder {f}"))
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_PNG) + struct.pack("<III", 0, 5, F.FMT_PARGB), it.expect_err(F.ERR_DIMS), "w 0"))
    recs.append(Rec(it.OP_ENCODE, 0, imgopt(F.IMGF_PNG) + it.surface_input(P, 7), it.expect_err(F.ERR_UNSUPPORTED), "surface fmt 7"))
    return recs, sizes


def convert_ex_checks(runner, suite, rng):
    """--convert-ex: img_load_ex (hint from the extension) + img_save_opt (format from the
    extension, atomic write)."""
    S = it.rand_bgra(rng, 19, 8, "mixed")
    src = runner.work / "cx_src.png"
    src.write_bytes(it.png_make(S[..., [2, 1, 0, 3]]))
    for ext, fmt in (("png", 1), ("bmp", 3), ("jpg", 2), ("gif", 4), ("tif", 5), ("xyz", 0)):
        dst = runner.work / f"cx_out.{ext}"
        if dst.exists():
            dst.unlink()
        p = runner.run(["--convert-ex", runner.rel(src), "--to", runner.rel(dst), "--quality", "90"], timeout=120)
        if fmt in (1, 2, 3, 4) or (fmt and has_cap(fmt, C.IFC_ENCODE)):
            ok = p.returncode == 0 and dst.exists()
            if ok:
                r = runner.one(it.OP_PROBE, 256, dst.read_bytes())
                ok = r[0] == fmt
            suite.check(ok, f"convert-ex -> .{ext}: rc {p.returncode} {p.stdout.strip()} {p.stderr.strip()}")
            if fmt in (1, 3) and ok:
                r2 = runner.one(it.OP_DECEX, 0, it.decex_input(dst.read_bytes()))
                suite.check(r2[0] == 0 and np.array_equal(it.parse_decex(r2[1])["px"], it.premul(S)), f"convert-ex .{ext} pixels")
        else:
            suite.check(p.returncode != 0 and not dst.exists(), f"convert-ex -> .{ext} should fail (rc {p.returncode})")
    p = runner.run(["--convert-ex", runner.rel(src), "--to", runner.rel(runner.work / "cx_s.png"), "--straight"], timeout=120)
    suite.check(p.returncode == 0 and "fmt 3" in p.stdout, f"convert-ex --straight: {p.stdout.strip()}")
    p = runner.run(["--convert-ex", runner.rel(runner.work / "missing.png"), "--to", runner.rel(runner.work / "x.png")], timeout=120)
    suite.check(p.returncode != 0 and "error -13" in (p.stdout + p.stderr), f"convert-ex missing file: {p.stderr.strip()}")


# ================================================================== fuzz

def suite_fuzz(rng, runner, count):
    F = C
    corpus = []
    S = it.rand_bgra(rng, 11, 7, "mixed")
    prof = icc_profile()
    corpus.append(("png", it.png_make(S[..., [2, 1, 0, 3]], extra_chunks=[
        it.png_chunk(b"pHYs", struct.pack(">IIB", 3780, 3780, 1)),
        it.png_chunk(b"iCCP", b"p\0\0" + zlib.compress(prof)),
        it.png_chunk(b"iTXt", b"XML:com.adobe.xmp\0\1\0\0\0" + zlib.compress(b"<x/>" * 20)),
        it.png_chunk(b"acTL", struct.pack(">II", 2, 1))], after_idat=[it.png_chunk(b"eXIf", it.exif_blob(orient=6))])))
    base = encode_file(runner, F.IMGF_JPEG, smooth_bgra(rng, 16, 16), F.FMT_PARGB, quality=75)
    corpus.append(("jpeg", it.jpeg_insert(base, [it.jpeg_seg(0xE1, b"Exif\0\0" + it.exif_blob(orient=5, xres=(72, 1), colorspace=1)),
                                                 xmp_app1(b"<x:xmpmeta/>")] + icc_segments_py(prof))))
    corpus.append(("gif", gif_anim([[0, 1, 2, 3, 3, 2], [1, 1, 1, 1, 1, 1]], 3, 2)))
    corpus.append(("bmp", bmp_v5(S, icc=prof)))
    corpus.append(("isobmff", ftyp_box(b"mif1", [b"mif1"]) + heif_meta(2, [(1, b"hvc1"), (2, b"av01")])))
    corpus.append(("tga", tga_header(4, 3) + bytes(48)))
    exif = it.exif_blob(orient=3, xres=(300, 1), yres=(300, 1), unit=2, colorspace=1, extra_ifd1=True)
    gz = gz_member(b"payload " * 100, flags=2 | 4 | 8 | 16, extra=b"ab\x02\x00xy", name=b"n", comment=b"c")
    recs = []
    allflags = F.IMGD_ICC | F.IMGD_EXIF | F.IMGD_XMP
    for i in range(count):
        k = i % 6
        if k < 3:
            nm, data = corpus[rng.randrange(len(corpus))]
            m = it.mutate(rng, data, keep=rng.choice([0, 4, 8]))
            fl = allflags | (F.IMGD_PROBE if k == 1 else 0) | (F.IMGD_STRAIGHT if k == 2 else 0)
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(m, fl, hint=rng.choice([0, 0, F.IMGF_TGA])), it.any_result, f"decex {nm} #{i}"))
        elif k == 3:
            nm, data = corpus[rng.randrange(len(corpus))]
            recs.append(Rec(it.OP_PROBE, 0, it.mutate(rng, data), it.any_result, f"probe {nm} #{i}"))
        elif k == 4:
            recs.append(Rec(it.OP_EXIFREAD, 0, it.mutate(rng, exif), it.any_result, f"exif #{i}"))
            recs.append(Rec(it.OP_ICCCOL, 0, it.mutate(rng, corpus[1][1], keep=2), it.any_result, f"icc #{i}"))
        else:
            recs.append(Rec(it.OP_GZDEC, 2000, it.mutate(rng, gz), it.any_result, f"gzip #{i}"))
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
        runner = it.Runner(target, out, work_name="imgmeta")
        core = it.Runner(target, out, extra=["--core-only"], work_name="imgmeta")
        load_caps(runner)

        def run(name, fn, r=runner):
            s = it.Suite(name, target)
            recs = fn()
            it.run_suite(r, s, recs)
            suites.append(s)
            return s
        if want("gzip"):
            run("gzip", lambda: suite_gzip(random.Random(a.seed)))
        if want("exif"):
            run("exif", lambda: suite_exif(random.Random(a.seed + 1)))
        if want("icc"):
            run("icc", lambda: suite_icc(random.Random(a.seed + 2), runner))
        if want("detect"):
            run("detect", lambda: suite_detect(random.Random(a.seed + 3), False))
            run("detect-core", lambda: suite_detect(random.Random(a.seed + 3), True), core)
        if want("probe"):
            run("probe", lambda: suite_probe(random.Random(a.seed + 4), runner))
        if want("fmtinfo"):
            run("fmtinfo", lambda: suite_fmtinfo(random.Random(a.seed + 5), False))
            run("fmtinfo-core", lambda: suite_fmtinfo(random.Random(a.seed + 5), True), core)
        if want("decex"):
            run("decex", lambda: suite_decex(random.Random(a.seed + 6), runner, False))
        if want("pixels"):
            run("pixels", lambda: suite_pixels(random.Random(a.seed + 7)))
        if want("encode"):
            s = it.Suite("encode", target)
            recs, sizes = suite_encode(random.Random(a.seed + 8), runner, False)
            it.run_suite(runner, s, recs)
            s.check(sizes.get(20, 0) < sizes.get(98, 0), f"JPEG q20 {sizes.get(20)} not smaller than q98 {sizes.get(98)}")
            convert_ex_checks(runner, s, random.Random(a.seed + 9))
            suites.append(s)
        if want("fuzz"):
            run("fuzz", lambda: suite_fuzz(random.Random(a.seed + 10), runner, a.fuzz))
    ok = it.report(suites, "imgmeta_check")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
