#!/usr/bin/env python3
"""pdf_render_check.py -- the pages drawn by lib/pdf's renderer (pdf::read, Document::render)
against Poppler's pdftoppm

The corpus is pdf_read_check.py's (the writer's sample, PDFs of every structure, the encrypted
fixtures, Ghostscript, LibreOffice, cairo, the system's PDFs) and pages made here for drawing:
  - paths: fill rules, curves, caps, joins, miter limits, dashes, hairlines, a stroke under a
    non-uniform matrix, clipping paths inside each other
  - colours: every colour space (Indexed, Separation with type 2 and type 4 functions, DeviceN,
    Lab, ICCBased, CalRGB, Separation /All)
  - shadings: axial, radial (with and without Extend), function-based, free-form and lattice
    triangles, Coons and tensor-product patches (also with bit widths that are not bytes),
    shading patterns
  - images: every bit depth, Decode, Indexed, stencil masks, colour-key and explicit masks, soft
    masks, inline images, turned images, enlarged and shrunk, JPEG (RGB, grey, CMYK), CCITT
    Group 3 and 4
  - transparency: alpha, the sixteen blend modes, soft masks (luminosity, alpha), groups
  - text: the 14 standard fonts, embedded Type 1, CFF, TrueType, OpenType and CID fonts, Type 3,
    every render mode
  - annotations with appearances, optional content, rotated and cropped pages, tiling patterns
  - pycairo: gradients, meshes, images with alpha, groups, blend modes, embedded fonts
Each page is drawn by `pdftest --render` and by `pdftoppm` at the same resolution; a pixel
counts as different when one of its colours lies further than --tol (48) outside the colours
of the other image's pixel and its eight neighbours (so a shift below a pixel is no
difference).  A page passes when at most --max (0.5) percent of its pixels differ.

  python tools/pdf_render_check.py [--exe build/linux/pdftest] [--dpi 72] [--keep DIR]
                                   [--diff] [--only NAME] [--pages 20]

--keep keeps the corpus and the images, --diff writes ours | Poppler | difference as PNG for
every page that fails (into --keep).  Exit code 1 when a page fails.
"""
import argparse
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pdf_read_check as prc  # noqa: E402
from pdf_read_check import Pdf  # noqa: E402

try:
    from PIL import Image, ImageChops, ImageFilter
except ImportError:  # pragma: no cover
    Image = None

ROOT = Path(__file__).resolve().parents[1]
URW = Path("/usr/share/fonts/type1/urw-base35")

# where Poppler draws differently, by file (and page): (the most pixels that may differ, why)
KNOWN = {
    "form_english.pdf": (2.5, "its rules are hundreds of rectangles 0.1 pt wide: Poppler's rasterizer counts every subpixel such a "
                              "rectangle touches and draws them darker than their area, ours draws their area"),
    "form_russian.pdf": (2.5, "the rules of form_english.pdf"),
    ("draw_patterns.pdf", 2): (5.0, "Poppler puts the cells of a turned tiling pattern off their grid (at 3000 points ours match the "
                                    "pattern's geometry 99.8 %, Poppler's 88 %)"),
}


# ------------------------------------------------------------------ a document of pages

class Doc:
    def __init__(self):
        self.p = Pdf()
        self.p.add(b"", 1)
        self.p.add(b"", 2)
        self.kids = []

    def obj(self, body):
        return self.p.add(body)

    def stream(self, dict_body, data, compress=True):
        return self.p.add(Pdf.stream(dict_body, data, compress))

    def page(self, content, res=b"", extra=b"", box=b"[0 0 400 400]"):
        c = self.stream(b"", content)
        self.kids.append(self.p.add(b"<< /Type /Page /Parent 2 0 R /MediaBox " + box + b" /Contents %d 0 R /Resources << " % c + res + b" >> " + extra + b">>"))

    def write(self, catalog=b""):
        self.p.objs[2] = b"<< /Type /Pages /Kids [" + b" ".join(b"%d 0 R" % k for k in self.kids) + b"] /Count %d >>" % len(self.kids)
        self.p.objs[1] = b"<< /Type /Catalog /Pages 2 0 R " + catalog + b">>"
        return self.p.write(1)


HELV = b"/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> >>"


def label(x, y, s, size=9):
    return b"BT /F1 %d Tf %g %g Td (%s) Tj ET\n" % (size, x, y, s.encode("cp1252"))


# ------------------------------------------------------------------ paths and colours

def paths(d):
    doc = Doc()
    c = b"""q 1 0 0 RG 0 0 1 rg 4 w 20 20 m 120 20 l 70 120 l h B
q 0 .5 0 rg 150 20 m 250 120 250 20 380 80 c 330 120 300 150 v 200 140 160 100 y h f* Q
q 0.8 0.5 0.1 rg 1 0 0 1 20 140 cm 0 0 m 60 0 l 60 40 l 0 40 l h 10 10 m 10 30 l 50 30 l 50 10 l h f Q
q 0.1 0.5 0.8 rg 1 0 0 1 100 140 cm 0 0 m 60 0 l 60 40 l 0 40 l h 10 10 m 10 30 l 50 30 l 50 10 l h f* Q
0 0 0 RG 10 w 0 J 20 200 m 120 200 l S 1 J 20 230 m 120 230 l S 2 J 20 260 m 120 260 l S
0 J 0 j 10 M 160 180 m 200 260 l 240 180 l S 1 j 260 180 m 300 260 l 340 180 l S
2 j 160 290 m 200 370 l 240 290 l S 0 j 1.5 M 260 290 m 300 370 l 340 290 l S
2 w [6 3 1 3] 0 d 0 J 20 300 m 140 300 l S [8 4] 5 d 1 J 20 312 m 140 312 l S [] 0 d
0 w 20 325 m 140 345 l S 0.5 w 20 335 m 140 355 l S
q 3 0 0 1 0 0 cm 1 w 7 370 m 45 370 l 45 390 l S Q
q 0 1 0 RG 6 w 1 j 200 150 m 260 150 l 230 120 l s Q
Q
"""
    doc.page(c)
    c = b"""q 50 50 300 300 re 120 120 160 160 re W* n 0.2 0.6 0.9 rg 0 0 400 400 re f Q
q 200 340 m 260 340 260 390 200 390 c 140 390 140 340 200 340 c W n 0.9 0.4 0.1 rg 0 0 400 400 re f
  1 1 1 rg 150 330 10 70 re f 180 330 10 70 re f 210 330 10 70 re f 240 330 10 70 re f Q
q 10 10 150 100 re W n q 60 40 150 100 re W n 0 0.6 0 rg 0 0 400 400 re f Q Q
q 1 0 0 1 300 60 cm 0.7 0.7 0 rg 0 0 m 40 0 l 40 40 l 0 40 l h 20 20 m 60 20 l 60 60 l 20 60 l h f* Q
"""
    doc.page(c)
    return doc.write()


def colours(d):
    doc = Doc()
    ps = doc.stream(b"/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]", b"{ dup 0.5 mul exch 1 exch sub 0.3 }", compress=False)
    psn = doc.stream(b"/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1]", b"{ 0 0 }", compress=False)
    samp = doc.stream(b"/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1] /Size [3] /BitsPerSample 8", bytes([255, 0, 0, 0, 255, 0, 0, 0, 255]))
    icc_path = Path("/usr/share/color/icc/ghostscript/srgb.icc")
    icc = doc.stream(b"/N 3 /Alternate /DeviceRGB", icc_path.read_bytes()) if icc_path.exists() else None
    cs = (b"/ColorSpace << /CS0 [/Indexed /DeviceRGB 3 <ff0000 00ff00 0000ff ffff00>] "
          b"/CS1 [/Separation /Spot /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.5 1 0] /N 1 >>] "
          b"/CS2 [/Separation /Spot2 /DeviceRGB %d 0 R] /CS3 [/DeviceN [/Cyan /Magenta] /DeviceCMYK %d 0 R] "
          b"/CS4 [/Lab << /WhitePoint [0.9505 1 1.089] /Range [-100 100 -100 100] >>] "
          b"/CS6 [/CalRGB << /WhitePoint [0.9505 1 1.089] >>] /CS7 [/Separation /All /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] "
          b"/CS8 [/Separation /Ramp /DeviceRGB %d 0 R] " % (ps, psn, samp))
    if icc:
        cs += b"/CS5 [/ICCBased %d 0 R] " % icc
    cs += b">>"
    rows = [b"0.5 g", b"0.2 0.6 0.9 rg", b"0 0 0 0.5 k", b"0.2 0.3 0.4 0.1 k", b"/CS0 cs 2 sc", b"/CS0 cs 3 sc", b"/CS1 cs 0.7 scn", b"/CS1 cs 0.3 scn",
            b"/CS2 cs 0.4 scn", b"/CS3 cs 0.5 0.8 scn", b"/CS4 cs 50 40 -30 scn", b"/CS4 cs 80 -60 50 scn", b"/CS6 cs 0.8 0.3 0.6 scn",
            b"/CS7 cs 0.6 scn", b"/CS8 cs 0.25 scn", b"/CS8 cs 0.75 scn"]
    if icc:
        rows.append(b"/CS5 cs 0.2 0.7 0.3 scn")
    c = bytearray()
    for i, r in enumerate(rows):
        x, y = 20 + (i % 4) * 95, 320 - (i // 4) * 70
        c += b"q " + r + b" %d %d 80 55 re f Q\n" % (x, y)
        c += b"q 0 0 0 RG 1 0 0 RG %d %d 80 55 re S Q\n" % (x, y) if i % 3 == 0 else b""
    c += b"q /CS2 CS 0.9 SCN 6 w 20 20 m 380 20 l S Q\n"
    doc.page(bytes(c), cs)
    return doc.write()


# ------------------------------------------------------------------ shadings and patterns

def mesh4(tris, ncomp):
    out = bytearray()
    for tri in tris:
        for k, (x, y, c) in enumerate(tri):
            out += struct.pack(">BHH", 0, int(x / 400 * 65535), int(y / 400 * 65535)) + bytes(int(v * 255) for v in c[:ncomp])
    return bytes(out)


def shadings(d):
    doc = Doc()
    f_rb = b"<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>"
    f_st = b"<< /FunctionType 3 /Domain [0 1] /Functions [<< /FunctionType 2 /Domain [0 1] /C0 [1 1 0] /C1 [0 0.6 0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0 0.6 0] /C1 [0.6 0 0.6] /N 2 >>] /Bounds [0.4] /Encode [0 1 0 1] >>"
    samp = doc.stream(b"/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1] /Size [5] /BitsPerSample 8", bytes([255, 255, 255, 255, 200, 0, 0, 160, 0, 0, 0, 160, 40, 40, 40]))
    fps = doc.stream(b"/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]", b"{ 2 copy add 2 div }", compress=False)
    sh = {
        b"S0": b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [40 0 180 0] /Function " + f_rb + b" /Extend [true true] >>",
        b"S1": b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [220 210 360 380] /Function " + f_st + b" >>",
        b"S2": b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [100 100 0 100 100 70] /Function %d 0 R /Extend [false true] >>" % samp,
        b"S3": b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [270 70 5 300 100 70] /Function " + f_rb + b" /Extend [true false] >>",
        b"S4": b"<< /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 1 0 1] /Matrix [150 0 0 150 30 220] /Function %d 0 R >>" % fps,
    }
    res = b"/Shading << " + b" ".join(b"/" + k + b" " + v for k, v in sh.items()) + b" >>"
    c = b"""q 20 200 180 180 re W n /S0 sh Q
q 210 200 170 180 re W n /S1 sh Q
q 20 20 180 160 re W n /S2 sh Q
q 210 20 170 160 re W n /S3 sh Q
"""
    doc.page(c, res)
    c = b"q 20 210 160 160 re W n /S4 sh Q\n"
    doc.page(c, res)
    # meshes: free-form, lattice, Coons, tensor
    tris = [[(30, 30, (1, 0, 0)), (180, 40, (0, 1, 0)), (90, 180, (0, 0, 1))], [(200, 30, (1, 1, 0)), (380, 60, (0, 1, 1)), (260, 190, (1, 0, 1))]]
    s4 = doc.stream(b"/ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 400 0 400 0 1 0 1 0 1]", mesh4(tris, 3))
    lat = bytearray()
    for j in range(3):
        for i in range(3):
            x, y = 30 + i * 70 + (10 if j == 1 else 0), 220 + j * 70
            lat += struct.pack(">HH", int(x / 400 * 65535), int(y / 400 * 65535)) + bytes([int(255 * i / 2), int(255 * j / 2), 128])
    s5 = doc.stream(b"/ShadingType 5 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 /VerticesPerRow 3 /Decode [0 400 0 400 0 1 0 1 0 1]", bytes(lat))
    pts = [(240, 220), (250, 260), (230, 300), (240, 340), (280, 350), (320, 360), (360, 340), (350, 300), (370, 260), (360, 220), (320, 230), (280, 210)]
    coons = bytearray(b"\x00")
    for x, y in pts:
        coons += struct.pack(">HH", int(x / 400 * 65535), int(y / 400 * 65535))
    coons += bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0])
    s6 = doc.stream(b"/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 400 0 400 0 1 0 1 0 1]", bytes(coons))
    tpts = pts + [(270, 250), (280, 320), (330, 320), (320, 260)]
    tens = bytearray(b"\x00")
    for x, y in tpts:
        tens += struct.pack(">HH", int(x / 400 * 65535), int((y - 200) / 400 * 65535))
    tens += bytes([0, 128, 255, 255, 128, 0, 0, 255, 0, 128, 0, 128])
    s7 = doc.stream(b"/ShadingType 7 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 400 0 400 0 1 0 1 0 1]", bytes(tens))
    res = b"/Shading << /M4 %d 0 R /M5 %d 0 R /M6 %d 0 R /M7 %d 0 R >>" % (s4, s5, s6, s7)
    doc.page(b"/M4 sh /M5 sh /M6 sh /M7 sh\n", res)
    # odd bit widths: 2-bit flags, 12-bit coordinates, 4-bit colours, each vertex and each patch
    # from a byte on
    def bits(fields):
        v, n = 0, 0
        for val, w in fields:
            v, n = v << w | val, n + w
        pad = -n % 8
        return (v << pad).to_bytes((n + pad) // 8, "big")
    odd4 = b"".join(bits([(0, 2), (int(x / 400 * 4095), 12), (int(y / 400 * 4095), 12)] + [(int(c * 15), 4) for c in col])
                    for tri in tris for (x, y, col) in tri)
    o4 = doc.stream(b"/ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 12 /BitsPerComponent 4 /BitsPerFlag 2 /Decode [0 400 0 400 0 1 0 1 0 1]", odd4)
    patch = [(0, 2)] + [f for x, y in pts for f in ((int(x / 400 * 4095), 12), (int((y - 200) / 400 * 4095), 12))] + [(v, 4) for v in (15, 0, 0, 0, 15, 0, 0, 0, 15, 15, 15, 0)]
    o6 = doc.stream(b"/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 12 /BitsPerComponent 4 /BitsPerFlag 2 /Decode [0 400 0 400 0 1 0 1 0 1]", bits(patch))
    doc.page(b"/O4 sh /O6 sh\n", b"/Shading << /O4 %d 0 R /O6 %d 0 R >>" % (o4, o6))
    # shading patterns fill a path and text
    pat = b"/Pattern << /P0 << /PatternType 2 /Shading << /ShadingType 3 /ColorSpace /DeviceRGB /Coords [0 0 0 0 0 120] /Function " + f_rb + b" /Extend [true true] >> /Matrix [1 0 0 1 200 200] >> /P1 << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 300 0] /Function " + f_st + b" >> /Matrix [1 0 0 1 50 0] /Background [0.9 0.9 0.9] >> >> " + HELV
    c = b"/Pattern cs /P0 scn 200 300 m 230 230 l 300 230 l 245 185 l 265 115 l 200 160 l 135 115 l 155 185 l 100 230 l 170 230 l h f\n"
    c += b"/Pattern cs /P1 scn BT /F1 48 Tf 30 40 Td (Muster) Tj ET 300 30 80 60 re f\n"
    doc.page(c, pat)
    return doc.write()


def patterns(d):
    doc = Doc()
    # steps of whole pixels at 72 and 150 dpi (Poppler repeats its tile by whole pixels: other steps drift)
    cell = doc.stream(b"/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 36 36] /XStep 36 /YStep 36 /Resources << >>", b"1 0 0 rg 18 18 m 29 18 l 29 29 l 18 29 l h f 0 0 1 rg 4 4 11 11 re f")
    cell2 = doc.stream(b"/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 24 24] /XStep 24 /YStep 24 /Resources << >>", b"1 0 0 rg 12 12 6 6 re f 0 0 1 rg 2 2 6 6 re f")
    rot = doc.stream(b"/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 16 10] /XStep 16 /YStep 10 /Matrix [0.866 0.5 -0.5 0.866 0 0] /Resources << >>", b"0 0.5 0 rg 0 0 8 5 re f")
    unc = doc.stream(b"/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 12 12] /XStep 12 /YStep 12 /Resources << >>", b"0 0 m 12 12 l 12 0 m 0 12 l S")
    gap = doc.stream(b"/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 6 6] /XStep 24 /YStep 24 /Resources << >>", b"0.6 0 0.6 rg 0 0 6 6 re f")
    form = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Matrix [1.5 0 0 1.5 250 30] /Resources << /Pattern << /P0 %d 0 R >> >>" % cell2, b"/Pattern cs /P0 scn 0 0 100 100 re f")
    res = b"/Pattern << /P0 %d 0 R /P1 %d 0 R /P2 %d 0 R /P3 %d 0 R >> /ColorSpace << /UP [/Pattern /DeviceRGB] >> /XObject << /Fm %d 0 R >>" % (cell, rot, unc, gap, form)
    c = b"""/Pattern cs /P0 scn 20 220 160 160 re f
/UP cs 1 0 0 /P2 scn 20 20 100 150 re f
/UP cs 0 0.4 1 /P2 scn 130 20 100 150 re f
/Pattern cs /P3 scn 200 200 60 80 re f
/Fm Do
"""
    doc.page(c, res)
    doc.page(b"/Pattern cs /P1 scn 300 300 m 380 300 380 380 300 380 c 220 380 220 300 300 300 c f\n", res)
    return doc.write()


# ------------------------------------------------------------------ images

def png_rows(w, h, f):
    return bytes(v for y in range(h) for x in range(w) for v in f(x, y))


def images(d):
    doc = Doc()
    im = {}
    rgb8 = png_rows(8, 8, lambda x, y: (x * 32, y * 32, 255 - x * 16) if (x + y) % 2 else (255, 255, 255))
    im[b"I0"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8", rgb8)
    bits = bytearray()
    for y in range(64):
        row = 0
        for x in range(64):
            row = row << 1 | ((x // 4 + y // 4) % 2)
        bits += row.to_bytes(8, "big")
    im[b"I1"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 1", bytes(bits))
    idx = bytearray()
    for y in range(16):
        for x in range(0, 16, 2):
            idx.append(((x + y) % 16) << 4 | ((x + 1 + y) % 16))
    pal = bytes(v for i in range(16) for v in (i * 16, 255 - i * 16, (i * 64) % 256))
    im[b"I2"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace [/Indexed /DeviceRGB 15 <" + pal.hex().encode() + b">] /BitsPerComponent 4", bytes(idx))
    rgb16 = b"".join(struct.pack(">HHH", x * 4000, y * 4000, 30000) for y in range(12) for x in range(12))
    im[b"I3"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 12 /Height 12 /ColorSpace /DeviceRGB /BitsPerComponent 16", rgb16)
    gray = bytes((x * 16 + y * 4) % 256 for y in range(16) for x in range(16))
    im[b"I4"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 8 /Decode [1 0]", gray)
    stencil = bytearray()
    for y in range(16):
        r = 0
        for x in range(16):
            r = r << 1 | (0 if (x - 8) ** 2 + (y - 8) ** 2 < 40 else 1)
        stencil += r.to_bytes(2, "big")
    im[b"I5"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 16 /Height 16 /ImageMask true /BitsPerComponent 1", bytes(stencil))
    ck = png_rows(8, 8, lambda x, y: (0, 0, 0) if x in (3, 4) else (200, 50 + y * 20, 50))
    im[b"I6"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask [0 10 0 10 0 10]", ck)
    mk = doc.stream(b"/Type /XObject /Subtype /Image /Width 32 /Height 32 /ImageMask true /BitsPerComponent 1",
                    bytes(b for y in range(32) for b in ((0xFF000000 if y < 16 else 0x0000FFFF) ^ (0xF0F0F0F0 if y % 8 < 4 else 0)).to_bytes(4, "big")))
    im[b"I7"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask %d 0 R" % mk, rgb8)
    sm = doc.stream(b"/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 8", bytes(min(255, x * 17) for y in range(16) for x in range(16)))
    im[b"I8"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask %d 0 R" % sm, rgb8)
    big = png_rows(120, 90, lambda x, y: ((x * 2) % 256, (y * 3) % 256, ((x + y) * 5) % 256) if (x // 6 + y // 6) % 2 else (40, 40, 40))
    im[b"I9"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 120 /Height 90 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Interpolate true", big)
    res = b"/XObject << " + b" ".join(b"/" + k + b" %d 0 R" % v for k, v in im.items()) + b" >> " + HELV
    c = b"""q 120 0 0 120 10 270 cm /I0 Do Q
q 24 0 0 24 140 350 cm /I0 Do Q
q 6 0 0 6 140 330 cm /I0 Do Q
q 40 0 0 40 180 330 cm /I1 Do Q
q 64 0 0 64 230 310 cm /I2 Do Q
q 60 0 0 60 310 310 cm /I3 Do Q
q 50 0 0 50 10 200 cm /I4 Do Q
q 1 0 0 rg 60 0 0 60 70 200 cm /I5 Do Q
q 0 0 0.6 rg 13 0 0 13 140 200 cm /I5 Do Q
q 0.9 0.9 0.2 rg 150 205 60 50 re f 60 0 0 60 150 200 cm /I6 Do Q
q 0.3 0.9 0.3 rg 220 205 60 50 re f 60 0 0 60 220 200 cm /I7 Do Q
q 0.2 0.2 0.8 rg 290 205 60 50 re f 60 0 0 60 290 200 cm /I8 Do Q
q 1 0 0 1 60 40 cm 0.866 0.5 -0.5 0.866 0 0 cm 100 0 0 70 0 0 cm /I9 Do Q
q 60 0 0 45 200 40 cm /I9 Do Q
q 250 0 0 180 280 10 cm /I9 Do Q
q 40 0 0 40 120 120 cm BI /W 4 /H 4 /CS /RGB /BPC 8 /F /AHx ID
ff000000ff000000ffffffff ff00ff00ffff00ffff808080 00000080808080ff00ffffff 0000ff00ff00ff000000ff00>
EI Q
"""
    doc.page(c, res)
    # JPEG and CCITT through PIL
    if Image:
        from io import BytesIO
        pim = Image.new("RGB", (64, 48))
        for y in range(48):
            for x in range(64):
                pim.putpixel((x, y), (x * 4, y * 5, 128 + (x - y) % 127))
        jb = BytesIO(); pim.save(jb, "JPEG", quality=90)
        gb = BytesIO(); pim.convert("L").save(gb, "JPEG", quality=90)
        cb = BytesIO(); pim.convert("CMYK").save(cb, "JPEG", quality=90)
        j = {}
        j[b"J0"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 64 /Height 48 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", jb.getvalue(), compress=False)
        j[b"J1"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 64 /Height 48 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode", gb.getvalue(), compress=False)
        j[b"J2"] = doc.stream(b"/Type /XObject /Subtype /Image /Width 64 /Height 48 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Filter /DCTDecode /Decode [1 0 1 0 1 0 1 0]", cb.getvalue(), compress=False)
        fax = Image.new("1", (96, 40), 1)
        for y in range(40):
            for x in range(96):
                if (x // 8 + y // 6) % 2 == 0 and (x + y) % 5 != 0 or (x - 48) ** 2 + (y - 20) ** 2 < 120:
                    fax.putpixel((x, y), 0)
        for comp, k, name in (("group4", -1, b"C4"), ("group3", 0, b"C3")):
            tb = BytesIO(); fax.save(tb, "TIFF", compression=comp)
            t = Image.open(BytesIO(tb.getvalue()))
            off, n = t.tag_v2[273][0], t.tag_v2[279][0]
            data = tb.getvalue()[off:off + n]
            j[name] = doc.stream(b"/Type /XObject /Subtype /Image /Width 96 /Height 40 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K %d /Columns 96 /Rows 40 /BlackIs1 true >>" % k, data, compress=False)
        res = b"/XObject << " + b" ".join(b"/" + k + b" %d 0 R" % v for k, v in j.items()) + b" >>"
        c = b"""q 128 0 0 96 10 290 cm /J0 Do Q
q 128 0 0 96 150 290 cm /J1 Do Q
q 128 0 0 96 10 180 cm /J2 Do Q
q 192 0 0 80 150 180 cm /C4 Do Q
q 192 0 0 80 150 90 cm /C3 Do Q
q 48 0 0 20 20 100 cm /C4 Do Q
"""
        doc.page(c, res)
    return doc.write()


# ------------------------------------------------------------------ transparency

def transparency(d):
    doc = Doc()
    modes = [b"Normal", b"Multiply", b"Screen", b"Overlay", b"Darken", b"Lighten", b"ColorDodge", b"ColorBurn", b"HardLight", b"SoftLight",
             b"Difference", b"Exclusion", b"Hue", b"Saturation", b"Color", b"Luminosity"]
    gs = b" ".join(b"/B%d << /BM /%s >>" % (i, m) for i, m in enumerate(modes))
    grad = b"<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>"
    lum = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 400 400] /Group << /S /Transparency /CS /DeviceGray >> /Resources << /Shading << /G " + grad + b" >> >>",
                     b"q 1 0 0 1 30 0 cm /G sh Q")
    alpha = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 400 400] /Group << /S /Transparency >> /Resources << /ExtGState << /H << /ca 0.4 >> >> >>",
                       b"0 g 250 40 m 330 40 330 120 250 120 c 170 120 170 40 250 40 c f /H gs 300 60 60 60 re f")
    grp = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 400 400] /Group << /S /Transparency >> /Resources << >>",
                     b"1 0 0 rg 20 150 80 80 re f 0 0 1 rg 60 190 80 80 re f")
    res = (b"/ExtGState << " + gs + b" /A << /ca 0.5 /CA 0.5 >> /L << /SMask << /S /Luminosity /G %d 0 R >> >> /M << /SMask << /S /Alpha /G %d 0 R >> >> /N << /SMask /None >> /H << /ca 0.5 >> >> /XObject << /G %d 0 R >>" % (lum, alpha, grp))
    c = bytearray()
    for i in range(16):
        x, y = 20 + (i % 4) * 95, 300 - (i // 4) * 30
        c += b"q 0.2 0.5 0.9 rg %d %d 60 25 re f /B%d gs 0.9 0.6 0.2 rg %d %d 60 25 re f Q\n" % (x, y, i, x + 25, y + 8)
    c += b"q /A gs 0 0.6 0 rg 20 20 60 60 re f 4 w 1 0 0 RG 50 50 60 60 re S Q\n"
    c += b"q /L gs 0.9 0.1 0.1 rg 30 90 100 40 re f Q\n"
    c += b"q /M gs 0.1 0.1 0.9 rg 160 30 220 110 re f Q\n"
    c += b"q /H gs /G Do Q\n"
    doc.page(bytes(c), res)
    return doc.write()


# ------------------------------------------------------------------ text and fonts

def afm_widths(afm, enc_names):
    w = {}
    for line in afm.read_text(encoding="latin-1").splitlines():
        if line.startswith("C "):
            parts = dict(p.strip().split(" ", 1) for p in line.split(";") if p.strip() and " " in p.strip())
            if "N" in parts and "WX" in parts:
                w[parts["N"]] = int(float(parts["WX"]))
    return [w.get(n, 0) for n in enc_names]


def win_names():
    """WinAnsiEncoding's glyph names (through gen_pdf_tables' reader)"""
    import gen_pdf_tables as g
    return g.read_encodings(g.gs_dir())["WinAnsiEncoding"]


def split_t1(data):
    i = data.index(b"eexec") + 5
    while data[i:i + 1] in (b"\r", b"\n", b" "):
        i += 1
    j = data.rindex(b"cleartomark")
    k = j
    zeros = 0
    while k > i and data[k - 1:k] in (b"0", b"\n", b"\r", b" "):
        k -= 1
        zeros += data[k:k + 1] == b"0"
    return data[:i], data[i:k], data[k:]


def ttf_cmap_gids(path, chars):
    """unicode -> glyph id from a TrueType font's (3,1) format 4 cmap; and glyph id -> advance in 1/1000"""
    b = path.read_bytes()
    n = struct.unpack(">H", b[4:6])[0]
    tab = {}
    for i in range(n):
        tag, _, off, ln = struct.unpack(">4sIII", b[12 + 16 * i:28 + 16 * i])
        tab[tag.decode()] = (off, ln)
    upem = struct.unpack(">H", b[tab["head"][0] + 18:tab["head"][0] + 20])[0]
    nhm = struct.unpack(">H", b[tab["hhea"][0] + 34:tab["hhea"][0] + 36])[0]
    hm = tab["hmtx"][0]
    adv = lambda g: round(struct.unpack(">H", b[hm + 4 * min(g, nhm - 1):hm + 4 * min(g, nhm - 1) + 2])[0] * 1000 / upem)
    co = tab["cmap"][0]
    gids = {}
    for i in range(struct.unpack(">H", b[co + 2:co + 4])[0]):
        pid, eid, off = struct.unpack(">HHI", b[co + 4 + 8 * i:co + 12 + 8 * i])
        if (pid, eid) != (3, 1):
            continue
        s = co + off
        if struct.unpack(">H", b[s:s + 2])[0] != 4:
            continue
        seg = struct.unpack(">H", b[s + 6:s + 8])[0] // 2
        ends = struct.unpack(f">{seg}H", b[s + 14:s + 14 + 2 * seg])
        starts = struct.unpack(f">{seg}H", b[s + 16 + 2 * seg:s + 16 + 4 * seg])
        deltas = struct.unpack(f">{seg}h", b[s + 16 + 4 * seg:s + 16 + 6 * seg])
        ro_at = s + 16 + 6 * seg
        ros = struct.unpack(f">{seg}H", b[ro_at:ro_at + 2 * seg])
        for ch in chars:
            c = ord(ch)
            for k in range(seg):
                if starts[k] <= c <= ends[k]:
                    if ros[k] == 0:
                        g = (c + deltas[k]) & 0xFFFF
                    else:
                        p = ro_at + 2 * k + ros[k] + 2 * (c - starts[k])
                        g = struct.unpack(">H", b[p:p + 2])[0]
                        g = (g + deltas[k]) & 0xFFFF if g else 0
                    gids[ch] = g
                    break
        break
    return gids, adv


def text(d):
    doc = Doc()
    std = ["Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique", "Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic",
           "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique"]
    fonts = b" ".join(b"/S%d << /Type /Font /Subtype /Type1 /BaseFont /%s /Encoding /WinAnsiEncoding >>" % (i, f.encode()) for i, f in enumerate(std))
    fonts += b" /SY << /Type /Font /Subtype /Type1 /BaseFont /Symbol >> /ZD << /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>"
    c = bytearray()
    for i, f in enumerate(std):
        c += b"BT /S%d 14 Tf 20 %d Td (%s: Hamburgefonstiv \xc4\xd6\xdc \xe4\xf6\xfc \xdf \x80) Tj ET\n" % (i, 370 - i * 24, f.encode())
    c += b"BT /SY 16 Tf 20 70 Td (abgdpqWSY \\245 \\253) Tj ET BT /ZD 16 Tf 20 45 Td (3456 nopq HIJK) Tj ET\n"
    doc.page(bytes(c), b"/Font << " + fonts + b" >>")
    # embedded fonts
    names = win_names()
    first, last = 32, 255
    res = []
    c = bytearray()
    y = 360
    t1 = URW / "NimbusSans-Regular.t1"
    if t1.exists():
        a, b2, z = split_t1(t1.read_bytes())
        ff = doc.stream(b"/Length1 %d /Length2 %d /Length3 %d" % (len(a), len(b2), len(z)), a + b2 + z)
        w = afm_widths(URW / "NimbusSans-Regular.afm", names[first:last + 1])
        fd = doc.obj(b"<< /Type /FontDescriptor /FontName /NimbusSans-Regular /Flags 32 /FontBBox [-200 -300 1200 1000] /ItalicAngle 0 /Ascent 729 /Descent -218 /CapHeight 729 /StemV 80 /FontFile %d 0 R >>" % ff)
        res.append(b"/T1 << /Type /Font /Subtype /Type1 /BaseFont /NimbusSans-Regular /FirstChar 32 /LastChar 255 /Widths [" + b" ".join(b"%d" % v for v in w) +
                   b"] /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /Adieresis /eacute] >> /FontDescriptor %d 0 R >>" % fd)
        c += b"BT /T1 18 Tf 20 %d Td (Type 1: ABC Hamburg \xc4\xe9\xdf \x80 fi) Tj ET\n" % y
        y -= 34
    otf = Path("/usr/share/fonts/opentype/urw-base35/NimbusSans-Regular.otf")
    if not otf.exists():
        cands = list(Path("/usr/share/fonts").rglob("NimbusSans-Regular.otf"))
        otf = cands[0] if cands else otf
    if otf.exists():
        ob = otf.read_bytes()
        n = struct.unpack(">H", ob[4:6])[0]
        cff = None
        for i in range(n):
            tag, _, off, ln = struct.unpack(">4sIII", ob[12 + 16 * i:28 + 16 * i])
            if tag == b"CFF ":
                cff = ob[off:off + ln]
        w = afm_widths(URW / "NimbusSans-Regular.afm", names[first:last + 1])
        if cff:
            ff = doc.stream(b"/Subtype /Type1C", cff)
            fd = doc.obj(b"<< /Type /FontDescriptor /FontName /NimbusSans-Regular /Flags 32 /FontBBox [-200 -300 1200 1000] /ItalicAngle 0 /Ascent 729 /Descent -218 /CapHeight 729 /StemV 80 /FontFile3 %d 0 R >>" % ff)
            res.append(b"/T2 << /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+NimbusSans-Regular /FirstChar 32 /LastChar 255 /Widths [" + b" ".join(b"%d" % v for v in w) +
                       b"] /Encoding /WinAnsiEncoding /FontDescriptor %d 0 R >>" % fd)
            c += b"BT /T2 18 Tf 20 %d Td (CFF: Gr\xfc\xdfe aus \xc4\xd6\xdc \x80 Ligatur) Tj ET\n" % y
            y -= 34
        ff = doc.stream(b"/Subtype /OpenType", ob)
        fd = doc.obj(b"<< /Type /FontDescriptor /FontName /NimbusSans-Regular /Flags 32 /FontBBox [-200 -300 1200 1000] /ItalicAngle 0 /Ascent 729 /Descent -218 /CapHeight 729 /StemV 80 /FontFile3 %d 0 R >>" % ff)
        res.append(b"/T3 << /Type /Font /Subtype /Type1 /BaseFont /NimbusSans-Regular /FirstChar 32 /LastChar 255 /Widths [" + b" ".join(b"%d" % v for v in w) +
                   b"] /Encoding /WinAnsiEncoding /FontDescriptor %d 0 R >>" % fd)
        c += b"BT /T3 18 Tf 20 %d Td (OpenType: Quality \xe9t\xe9 \xbd) Tj ET\n" % y
        y -= 34
    lib = Path("/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf")
    if lib.exists():
        import gen_pdf_tables as g
        enc = win_names()[first:last + 1]
        agl = g.read_agl(g.gs_dir() / "gs_agl.ps")
        uni = [chr(agl[n][0]) if n in agl else "\0" for n in enc]
        gids, adv = ttf_cmap_gids(lib, uni)
        w = [adv(gids.get(ch, 0)) for ch in uni]
        ff = doc.stream(b"/Length1 %d" % lib.stat().st_size, lib.read_bytes())
        fd = doc.obj(b"<< /Type /FontDescriptor /FontName /LiberationSerif /Flags 34 /FontBBox [-500 -300 1300 1000] /ItalicAngle 0 /Ascent 891 /Descent -216 /CapHeight 700 /StemV 80 /FontFile2 %d 0 R >>" % ff)
        res.append(b"/T4 << /Type /Font /Subtype /TrueType /BaseFont /LiberationSerif /FirstChar 32 /LastChar 255 /Widths [" + b" ".join(b"%d" % v for v in w) +
                   b"] /Encoding /WinAnsiEncoding /FontDescriptor %d 0 R >>" % fd)
        c += b"BT /T4 18 Tf 20 %d Td (TrueType: Serifen \xe4\xf6\xfc \x80 \x93Zitat\x94) Tj ET\n" % y
        y -= 34
    dj = Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    if dj.exists():
        s = "CID: Καλημέρα мир ∑ √ ≈ →"
        gids, adv = ttf_cmap_gids(dj, s)
        ff = doc.stream(b"", dj.read_bytes())
        fd = doc.obj(b"<< /Type /FontDescriptor /FontName /DejaVuSans /Flags 32 /FontBBox [-1000 -400 2000 1200] /ItalicAngle 0 /Ascent 928 /Descent -236 /CapHeight 729 /StemV 80 /FontFile2 %d 0 R >>" % ff)
        wl = b" ".join(b"%d [%d]" % (gids[ch], adv(gids[ch])) for ch in sorted(set(s), key=lambda ch: gids[ch]))
        res.append(b"/T5 << /Type /Font /Subtype /Type0 /BaseFont /DejaVuSans /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /DejaVuSans "
                   b"/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor %d 0 R /W [" % fd + wl + b"] /CIDToGIDMap /Identity >>] >>")
        c += b"BT /T5 18 Tf 20 %d Td <" % y + "".join("%04x" % gids[ch] for ch in s).encode() + b"> Tj ET\n"
        y -= 34
    doc.page(bytes(c), b"/Font << " + b" ".join(res) + b" >>")
    # render modes, spacing, turned text
    c = b"""BT /F1 30 Tf 20 350 Td 1 Tr 0.5 w (Stroke) Tj 2 Tr 1 0 0 rg (Both) Tj 3 Tr (Invisible) Tj 0 Tr 0 g ET
q BT /F1 40 Tf 20 290 Td 7 Tr (CLIPPED) Tj ET 0 0 1 rg 20 280 300 60 re f 1 1 0 rg 0 300 400 5 re f Q
q BT /F1 30 Tf 20 230 Td 4 Tr 0 0.5 0 rg (FillClip) Tj ET 1 0 0 rg 0 240 400 6 re f Q
BT /F1 14 Tf 20 200 Td 3 Tc (Character spacing) Tj 0 Tc 0 -18 Td 8 Tw (Word spacing here) Tj 0 Tw 0 -18 Td 150 Tz (Horizontal scaling) Tj 100 Tz
0 -18 Td (Rise ) Tj 5 Ts (up) Tj -4 Ts (down) Tj 0 Ts 0 -18 Td [(Kern) 500 (ing) -300 (TJ)] TJ ET
BT /F1 16 Tf 0.866 0.5 -0.5 0.866 250 60 Tm (Turned 30 degrees) Tj ET
BT /F1 16 Tf 1 0 0.4 1 230 30 Tm (Slanted text) Tj ET
"""
    doc.page(c, HELV)
    # a Type 3 font: coloured and uncoloured glyphs
    sq = doc.stream(b"", b"1000 0 0 0 750 750 d1 0 0 750 750 re f")
    tri = doc.stream(b"", b"1000 0 d0 1 0 0 rg 0 0 m 750 0 l 375 750 l h f")
    circ = doc.stream(b"", b"800 0 0 0 700 700 d1 350 0 m 543 0 700 157 700 350 c 700 543 543 700 350 700 c 157 700 0 543 0 350 c 0 157 157 0 350 0 c f")
    t3 = (b"/F3 << /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /sq %d 0 R /tri %d 0 R /circ %d 0 R >> "
          b"/Encoding << /Type /Encoding /Differences [65 /sq /tri /circ] >> /FirstChar 65 /LastChar 67 /Widths [1000 1000 800] >>" % (sq, tri, circ))
    c = b"BT /F3 40 Tf 20 300 Td 0 0 1 rg (ABCABC) Tj ET BT /F3 20 Tf 0.7 0.7 -0.7 0.7 60 100 Tm 0 0.6 0 rg (CBA) Tj ET\n"
    doc.page(c, b"/Font << " + t3 + b" >>")
    return doc.write()


# ------------------------------------------------------------------ annotations, optional content, pages

def annots(d):
    doc = Doc()
    ap1 = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 100 50] /Resources << >>", b"0.2 0.4 0.9 rg 0 0 100 50 re f 1 1 0 RG 4 w 2 2 96 46 re S")
    ap2 = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 80 40] /Matrix [0.866 0.5 -0.5 0.866 0 0] /Resources << " + HELV + b" >>",
                     b"0.8 0 0 rg 0 0 80 40 re f BT /F1 14 Tf 1 g 8 14 Td (STAMP) Tj ET")
    on = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 20 20] /Resources << >>", b"0 g 2 2 16 16 re S 4 w 5 10 m 9 5 l 15 16 l S")
    off = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 20 20] /Resources << >>", b"0 g 2 2 16 16 re S")
    a = [
        doc.obj(b"<< /Type /Annot /Subtype /Square /Rect [20 300 120 350] /AP << /N %d 0 R >> >>" % ap1),
        doc.obj(b"<< /Type /Annot /Subtype /Stamp /Rect [150 250 300 380] /AP << /N %d 0 R >> >>" % ap2),
        doc.obj(b"<< /Type /Annot /Subtype /Square /Rect [20 200 220 220] /F 2 /AP << /N %d 0 R >> >>" % ap1),
        doc.obj(b"<< /Type /Annot /Subtype /Square /Rect [20 170 220 190] /F 32 /AP << /N %d 0 R >> >>" % ap1),
        doc.obj(b"<< /Type /Annot /Subtype /Widget /FT /Btn /Rect [20 100 60 140] /AS /On /AP << /N << /On %d 0 R /Off %d 0 R >> >> >>" % (on, off)),
        doc.obj(b"<< /Type /Annot /Subtype /Widget /FT /Btn /Rect [80 100 120 140] /AS /Off /AP << /N << /On %d 0 R /Off %d 0 R >> >> >>" % (on, off)),
        doc.obj(b"<< /Type /Annot /Subtype /Square /Rect [250 40 380 200] /AP << /N %d 0 R >> >>" % ap1),
    ]
    doc.page(b"0.9 g 0 0 400 400 re f\n", b"", b"/Annots [" + b" ".join(b"%d 0 R" % x for x in a) + b"]")
    return doc.write()


def optional(d):
    doc = Doc()
    g1 = doc.obj(b"<< /Type /OCG /Name (An) >>")
    g2 = doc.obj(b"<< /Type /OCG /Name (Aus) >>")
    md = doc.obj(b"<< /Type /OCMD /OCGs [%d 0 R %d 0 R] /P /AllOn >>" % (g1, g2))
    md2 = doc.obj(b"<< /Type /OCMD /OCGs [%d 0 R %d 0 R] /P /AnyOn >>" % (g1, g2))
    xo = doc.stream(b"/Type /XObject /Subtype /Form /BBox [0 0 400 400] /OC %d 0 R /Resources << >>" % g2, b"0 1 0 rg 250 250 100 100 re f")
    res = b"/Properties << /on %d 0 R /off %d 0 R /all %d 0 R /any %d 0 R >> /XObject << /X %d 0 R >>" % (g1, g2, md, md2, xo)
    c = b"""/OC /on BDC 1 0 0 rg 20 250 100 100 re f EMC
/OC /off BDC 0 0 1 rg 130 250 100 100 re f EMC
/OC /all BDC 0 1 1 rg 20 130 100 100 re f EMC
/OC /any BDC 1 0 1 rg 130 130 100 100 re f /OC /off BDC 0 g 140 140 20 20 re f EMC 0.5 g 170 140 20 20 re f EMC
/X Do
"""
    doc.page(c, res)
    return doc.write(b"/OCProperties << /OCGs [%d 0 R %d 0 R] /D << /OFF [%d 0 R] >> >>" % (g1, g2, g2))


def pages(d):
    doc = Doc()
    c = b"0 0 1 rg 20 20 200 100 re f 1 0 0 rg 20 300 50 50 re f " + label(30, 200, "Seite gedreht", 20)
    for rot in (90, 180, 270):
        doc.page(c, HELV, b"/Rotate %d" % rot, box=b"[0 0 400 400]")
    doc.page(c, HELV, b"/CropBox [10 10 300 380]", box=b"[0 0 400 400]")
    doc.page(c, HELV, b"", box=b"[-100 -50 300 450]")
    return doc.write()


PS_DRAW = r"""%!PS-Adobe-3.0
%%BoundingBox: 0 0 400 400
<< /PageSize [400 400] >> setpagedevice
% shadings
gsave 20 280 120 100 rectclip
<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [20 0 140 0] /Extend [true true]
   /Function << /FunctionType 2 /Domain [0 1] /C0 [0.9 0.1 0.1] /C1 [0.1 0.2 0.9] /N 1 >> >> shfill grestore
gsave 150 280 110 100 rectclip
<< /ShadingType 3 /ColorSpace /DeviceCMYK /Coords [205 330 0 205 330 55] /Extend [false true]
   /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 1 0] /C1 [1 0.3 0 0.2] /N 1 >> >> shfill grestore
gsave
<< /ShadingType 4 /ColorSpace /DeviceRGB /DataSource [0 270 280 1 0 0  0 390 280 0 1 0  0 330 380 0 0 1] >> shfill grestore
% a tiling pattern
<< /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 12 12] /XStep 12 /YStep 12
   /PaintProc { pop 0 0.5 0 setrgbcolor 0 0 6 6 rectfill 1 0.5 0 setrgbcolor 6 6 6 6 rectfill } >> matrix makepattern setpattern
20 160 120 100 rectfill
% an image and a stencil mask
gsave 150 160 translate 100 100 scale
16 16 8 [16 0 0 -16 0 16] { <
""" + "".join("%02x%02x%02x" % ((x * 16) % 256, (y * 16) % 256, ((x ^ y) * 16) % 256) for y in range(16) for x in range(16)) + r""" > } false 3 colorimage grestore
gsave 0.8 0 0.4 setrgbcolor 270 160 translate 100 100 scale
16 16 true [16 0 0 -16 0 16] { <""" + "".join("%04x" % (0xFFFF ^ (((1 << (16 - abs(8 - y) * 2)) - 1) << (abs(8 - y)))) for y in range(16)) + r"""> } imagemask grestore
% a Type 3 font
8 dict begin /FontType 3 def /FontMatrix [0.001 0 0 0.001 0 0] def /FontBBox [0 0 1000 1000] def
/Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for Encoding 65 /box put Encoding 66 /tri put
/CharProcs 3 dict def CharProcs begin /.notdef {} def /box { 0 0 800 800 rectfill } def
/tri { 0 0 moveto 800 0 lineto 400 800 lineto closepath fill } def end
/BuildGlyph { 1000 0 0 0 1000 1000 setcachedevice exch /CharProcs get exch get exec } def
/BuildChar { 1 index /Encoding get exch get 1 index /BuildGlyph get exec } def
currentdict end /Shapes exch definefont pop
/Shapes findfont 24 scalefont setfont 0 0 0.6 setrgbcolor 20 120 moveto (ABAB) show
% CMYK, dashes, even-odd clipping, turned text
0.8 0.1 0 0.1 setcmykcolor 4 setlinewidth [10 5] 0 setdash 150 130 moveto 380 130 lineto stroke [] 0 setdash
gsave newpath 150 20 100 80 rectpath 170 40 60 40 rectpath eoclip 0 0.6 0.3 setrgbcolor 0 0 400 400 rectfill grestore
/Helvetica-Bold findfont 18 scalefont setfont 0 setgray
gsave 280 30 translate 30 rotate 0 0 moveto (Gedreht) show grestore
20 30 moveto (Ghostscript) show
showpage
"""


def ghostscript_draw(d):
    ps = d / "draw_gs.ps"
    ps.write_text(PS_DRAW.replace("rectpath", "4 copy pop pop moveto 1 index 0 rlineto 0 exch rlineto neg 0 rlineto closepath pop pop"))
    f = d / "draw_gs.pdf"
    r = subprocess.run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite", f"-sOutputFile={f}", str(ps)], capture_output=True, timeout=120)
    if r.returncode != 0:
        print("pdf_render_check: Ghostscript failed on draw_gs.ps:", r.stderr.decode(errors="replace")[:300])
        return None
    return f


FODG = """<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.graphics">
<office:styles>
 <draw:gradient draw:name="lin" draw:style="linear" draw:start-color="#cc2222" draw:end-color="#2244cc" draw:angle="300" draw:border="0%"/>
 <draw:gradient draw:name="rad" draw:style="radial" draw:cx="40%" draw:cy="40%" draw:start-color="#ffee00" draw:end-color="#008844" draw:border="0%"/>
 <draw:opacity draw:name="fade" draw:style="linear" draw:start="100%" draw:end="0%" draw:angle="0"/>
 <draw:stroke-dash draw:name="dash" draw:style="rect" draw:dots1="1" draw:dots1-length="0.4cm" draw:distance="0.2cm"/>
 <draw:marker draw:name="arrow" svg:viewBox="0 0 20 30" svg:d="M10 0l-10 30h20z"/>
</office:styles>
<office:automatic-styles>
 <style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="14cm" fo:page-height="14cm" fo:margin-top="0cm" fo:margin-bottom="0cm" fo:margin-left="0cm" fo:margin-right="0cm"/></style:page-layout>
 <style:style style:name="g1" style:family="graphic"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="lin" draw:stroke="none"/></style:style>
 <style:style style:name="g2" style:family="graphic"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="rad" draw:stroke="solid" svg:stroke-color="#000000" svg:stroke-width="0.05cm"/></style:style>
 <style:style style:name="g3" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#3366ff" draw:opacity="50%" draw:stroke="none"/></style:style>
 <style:style style:name="g4" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#ff6633" draw:opacity-name="fade" draw:stroke="none"/></style:style>
 <style:style style:name="g5" style:family="graphic"><style:graphic-properties draw:fill="none" draw:stroke="dash" draw:stroke-dash="dash" svg:stroke-color="#226622" svg:stroke-width="0.08cm" draw:marker-end="arrow" draw:marker-end-width="0.4cm"/></style:style>
 <style:style style:name="g6" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#eeeeee" draw:stroke="solid" svg:stroke-color="#444444" draw:shadow="visible" draw:shadow-offset-x="0.2cm" draw:shadow-offset-y="0.2cm" draw:shadow-color="#888888"/></style:style>
 <style:style style:name="p1" style:family="paragraph"><style:text-properties fo:font-size="16pt" style:font-name="Liberation Sans" fo:color="#000000"/></style:style>
</office:automatic-styles>
<office:master-styles><style:master-page style:name="Default" style:page-layout-name="pm"/></office:master-styles>
<office:body><office:drawing><draw:page draw:name="p1" draw:master-page-name="Default">
 <draw:rect draw:style-name="g1" svg:x="0.5cm" svg:y="0.5cm" svg:width="6cm" svg:height="3cm"/>
 <draw:ellipse draw:style-name="g2" svg:x="7.5cm" svg:y="0.5cm" svg:width="6cm" svg:height="4cm"/>
 <draw:rect draw:style-name="g3" svg:x="1cm" svg:y="4.5cm" svg:width="4cm" svg:height="3cm"/>
 <draw:ellipse draw:style-name="g3" svg:x="3cm" svg:y="5.5cm" svg:width="4cm" svg:height="3cm"/>
 <draw:rect draw:style-name="g4" svg:x="8cm" svg:y="5cm" svg:width="5cm" svg:height="2.5cm"/>
 <draw:line draw:style-name="g5" svg:x1="1cm" svg:y1="9.5cm" svg:x2="12cm" svg:y2="10.5cm"/>
 <draw:custom-shape draw:style-name="g6" svg:x="1cm" svg:y="11cm" svg:width="5cm" svg:height="2cm"><text:p text:style-name="p1">Schatten &#x2013; Gr&#xFC;&#xDF;e</text:p>
  <draw:enhanced-geometry draw:type="round-rectangle" svg:viewBox="0 0 21600 21600"/></draw:custom-shape>
</draw:page></office:drawing></office:body></office:document>
"""


def libreoffice_draw(d):
    src = d / "draw_lo.fodg"
    src.write_text(FODG, encoding="utf-8")
    env = dict(os.environ, HOME=str(d))
    subprocess.run(["soffice", "--headless", "--norestore", "--convert-to", "pdf:draw_pdf_Export", "--outdir", str(d), str(src)], capture_output=True, timeout=180, env=env)
    f = d / "draw_lo.pdf"
    return f if f.exists() else None


def pycairo(d):
    try:
        import cairo
    except ImportError:
        return None
    f = d / "pycairo.pdf"
    s = cairo.PDFSurface(str(f), 400, 400)
    cr = cairo.Context(s)
    lg = cairo.LinearGradient(20, 0, 380, 0)
    lg.add_color_stop_rgb(0, 1, 0, 0)
    lg.add_color_stop_rgb(0.5, 1, 1, 0)
    lg.add_color_stop_rgb(1, 0, 0, 1)
    cr.rectangle(20, 20, 360, 60)
    cr.set_source(lg)
    cr.fill()
    rg = cairo.RadialGradient(100, 160, 5, 120, 170, 60)
    rg.add_color_stop_rgba(0, 1, 1, 1, 1)
    rg.add_color_stop_rgba(1, 0, 0.5, 0, 0.6)
    cr.arc(110, 170, 60, 0, 2 * math.pi)
    cr.set_source(rg)
    cr.fill()
    m = cairo.MeshPattern()
    m.begin_patch()
    m.move_to(220, 110)
    m.curve_to(260, 90, 300, 130, 360, 110)
    m.line_to(370, 220)
    m.curve_to(320, 240, 260, 200, 220, 230)
    m.line_to(220, 110)
    for i, col in enumerate([(1, 0, 0), (0, 1, 0), (0, 0, 1), (1, 1, 0)]):
        m.set_corner_color_rgb(i, *col)
    m.end_patch()
    cr.set_source(m)
    cr.paint()
    img = cairo.ImageSurface(cairo.FORMAT_ARGB32, 40, 40)
    ic = cairo.Context(img)
    ic.set_source_rgba(0.1, 0.6, 0.9, 0.8)
    ic.arc(20, 20, 18, 0, 2 * math.pi)
    ic.fill()
    cr.save()
    cr.translate(20, 250)
    cr.scale(2, 2)
    cr.set_source_surface(img, 0, 0)
    cr.paint()
    cr.restore()
    cr.push_group()
    cr.set_source_rgb(0.8, 0.2, 0.2)
    cr.rectangle(120, 250, 80, 80)
    cr.fill()
    cr.set_source_rgb(0.2, 0.2, 0.8)
    cr.rectangle(150, 280, 80, 80)
    cr.fill()
    cr.pop_group_to_source()
    cr.paint_with_alpha(0.5)
    cr.set_source_rgb(0.2, 0.7, 0.3)
    cr.rectangle(250, 250, 120, 60)
    cr.fill()
    cr.set_operator(cairo.OPERATOR_MULTIPLY)
    cr.set_source_rgb(0.9, 0.5, 0.9)
    cr.rectangle(280, 270, 100, 60)
    cr.fill()
    cr.set_operator(cairo.OPERATOR_OVER)
    cr.set_source_rgb(0, 0, 0)
    cr.set_line_width(3)
    cr.set_dash([8, 4])
    cr.move_to(20, 360)
    cr.line_to(380, 380)
    cr.stroke()
    cr.set_dash([])
    cr.select_font_face("DejaVu Serif", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
    cr.set_font_size(22)
    cr.move_to(20, 230)
    cr.show_text("Kairo: Größe ≠ Γ")
    cr.show_page()
    s.finish()
    return f


# ------------------------------------------------------------------ comparison

def diff_metric(a, b, tol):
    """the share of pixels that differ by more than tol from every pixel of the other image's 3x3
    neighbourhood (both ways), and the mean difference"""
    def outside(x, y):
        lo, hi = y.filter(ImageFilter.MinFilter(3)), y.filter(ImageFilter.MaxFilter(3))
        over = ImageChops.subtract(x, hi)
        under = ImageChops.subtract(lo, x)
        m = ImageChops.lighter(over, under)
        return m
    m = ImageChops.lighter(outside(a, b), outside(b, a))
    bands = m.split()
    worst = bands[0]
    for bnd in bands[1:]:
        worst = ImageChops.lighter(worst, bnd)
    hist = worst.histogram()
    n = a.width * a.height
    bad = sum(hist[tol + 1:]) / n
    dh = ImageChops.difference(a, b).convert("L").histogram()
    mean = sum(i * c for i, c in enumerate(dh)) / n
    return bad, mean, worst


def render_pair(exe, f, pw, opt, dpi, out, npages):
    ours = out / "ours"
    ref = out / "ref"
    for p in (ours, ref):
        shutil.rmtree(p, ignore_errors=True)
        p.mkdir(parents=True)
    t0 = time.time()
    args = [exe, "--render", str(f), "--out", str(ours / "p"), "--dpi", str(dpi), "--last", str(npages)] + (["--password", pw] if pw else [])
    r = subprocess.run(args, capture_output=True, timeout=900)
    t1 = time.time() - t0
    t0 = time.time()
    pargs = ["pdftoppm", "-cropbox", "-r", str(dpi), "-l", str(npages)] + ([opt, pw] if pw else []) + [str(f), str(ref / "p")]
    subprocess.run(pargs, capture_output=True, timeout=900)
    t2 = time.time() - t0
    refs = {}
    for p in ref.iterdir():
        n = int(p.stem.rsplit("-", 1)[1])
        refs[n] = p
    return r, t1, t2, refs, ours


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "build/linux/pdftest"))
    ap.add_argument("--dpi", type=float, default=72)
    ap.add_argument("--keep")
    ap.add_argument("--diff", action="store_true")
    ap.add_argument("--only")
    ap.add_argument("--pages", type=int, default=20)
    ap.add_argument("--tol", type=int, default=48)
    ap.add_argument("--max", type=float, default=0.5)
    a = ap.parse_args()
    if Image is None:
        print("pdf_render_check: Pillow is not installed")
        return 1
    if not shutil.which("pdftoppm"):
        print("pdf_render_check: pdftoppm (Poppler) is not installed")
        return 1
    d = Path(a.keep) if a.keep else Path(tempfile.mkdtemp(prefix="pdf_render_check_"))
    d.mkdir(parents=True, exist_ok=True)
    corpus = []
    for name, fn in (("paths", paths), ("colours", colours), ("shadings", shadings), ("patterns", patterns), ("images", images),
                     ("transparency", transparency), ("text", text), ("annots", annots), ("optional", optional), ("pages", pages)):
        f = d / f"draw_{name}.pdf"
        f.write_bytes(fn(d))
        corpus.append((f, None, None))
    for extra in (pycairo(d), ghostscript_draw(d) if shutil.which("gs") else None, libreoffice_draw(d) if shutil.which("soffice") else None):
        if extra:
            corpus.append((extra, None, None))
    sample = d / "sample.pdf"
    subprocess.run([a.exe, "--sample", str(sample)], capture_output=True)
    made = [sample] + prc.made_here(d)
    corpus += [(f, None, None) for f in made if f.name not in ("no_xref.pdf", "cut.pdf")]
    for f in sorted(prc.FIXTURES.glob("*.pdf")):
        corpus.append((f, "owner", "-opw"))
    if shutil.which("gs"):
        corpus += [c for c in prc.ghostscript(d, [m for m in made if m.name not in ("cut.pdf", "no_xref.pdf")]) if c[2] != "-opw"]
    if shutil.which("soffice"):
        corpus += [c for c in prc.libreoffice(d) if c[2] != "-opw"]
    if shutil.which("pdftocairo"):
        corpus += prc.cairo(d, [p for p in (sample, d / "type0.pdf", d / "page.pdf", d / "draw_text.pdf", d / "draw_shadings.pdf") if p.exists()])
    corpus += [(f, None, None) for f in prc.system_pdfs()]
    if a.only:
        corpus = [c for c in corpus if a.only in c[0].name]
    failed = 0
    tours = tref = 0.0
    work = d / "render"
    for f, pw, opt in corpus:
        name = f.name if f.parent in (d, prc.FIXTURES) else str(f)
        r, t1, t2, refs, ours = render_pair(a.exe, f, pw, opt, a.dpi, work, a.pages)
        tours += t1
        tref += t2
        if r.returncode not in (0, 1):
            failed += 1
            print(f"FAIL {name}: pdftest crashed (exit {r.returncode})")
            continue
        worst = (0.0, 0.0, 0)
        problems = []
        for n, rp in sorted(refs.items()):
            op = ours / f"p-{n}.ppm"
            if not op.exists():
                problems.append(f"page {n}: not drawn ({r.stdout.decode(errors='replace').strip()[:120]})")
                continue
            ia = Image.open(op).convert("RGB")
            ib = Image.open(rp).convert("RGB")
            if ia.size != ib.size:
                problems.append(f"page {n}: size {ia.size} vs Poppler {ib.size}")
                continue
            bad, mean, m = diff_metric(ia, ib, a.tol)
            known = KNOWN.get((f.name, n)) or KNOWN.get(f.name)
            limit = max(a.max, known[0]) if known else a.max
            if bad * 100 > limit:
                problems.append(f"page {n}: {bad * 100:.2f}% of the pixels differ (mean {mean:.2f})")
                if a.diff and a.keep:
                    w, h = ia.size
                    side = Image.new("RGB", (w * 3, h), "white")
                    side.paste(ia, (0, 0))
                    side.paste(ib, (w, 0))
                    side.paste(m.point(lambda v: 0 if v > a.tol else 255).convert("RGB"), (2 * w, 0))
                    side.save(Path(a.keep) / f"diff_{f.stem}_{n}.png")
            if bad > worst[0]:
                worst = (bad, mean, n)
        tag = f" [{opt} {pw}]" if opt else ""
        if problems:
            failed += 1
            print(f"FAIL {name}{tag}: {len(refs)} pages, {t1:.2f} s (Poppler {t2:.2f} s)")
            for p in problems[:8]:
                print(f"     {p}")
        else:
            note = KNOWN.get(f.name) or next((v for k, v in KNOWN.items() if isinstance(k, tuple) and k[0] == f.name), None)
            print(f"ok   {name}{tag}: {len(refs)} pages, the most different page {worst[0] * 100:.3f}% (mean {worst[1]:.2f}), {t1:.2f} s (Poppler {t2:.2f} s)"
                  + (f" -- {note[1]}" if note else ""))
    print(f"pdf_render_check: {len(corpus) - failed} of {len(corpus)} files agree with pdftoppm at {a.dpi:g} dpi; drawing took {tours:.1f} s (pdftoppm {tref:.1f} s)"
          + ("" if a.keep else f" (corpus in {d}, removed)"))
    if not a.keep:
        shutil.rmtree(d, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
