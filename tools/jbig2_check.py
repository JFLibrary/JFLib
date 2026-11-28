#!/usr/bin/env python3
"""jbig2_check.py -- lib/img_jbig2 (JBIG2) against jbig2dec (through Ghostscript) and Poppler,
on SerenityOS's JBIG2 test files, through `codectest --jbig2`

Suites
  serenity  SerenityOS's JBIG2 test files (BSD-2-Clause, fetched once at a pinned commit into
            build/vectors/, not in git): generic regions with every template, adaptive pixels and
            TPGDON, MMR, refinement with TPGRON, symbol dictionaries and text regions in arithmetic
            and Huffman coding (standard and custom tables, refinement and aggregate coding,
            transposed, every reference corner), halftone regions (skip, grids, 10 bits per
            pixel), striped pages of unknown height, random-access files, every combination
            operator, the example of Annex H.  Each file is decoded as a file (every page), and
            its first page is put into a PDF the way PDF embeds JBIG2 (the segments without the
            file header, those of page 0 in JBIG2Globals) and drawn by Ghostscript (jbig2dec) and
            by pdftoppm (Poppler) at 72 dpi; JBIG2 is lossless, so ours must equal theirs bit for
            bit.  Poppler's decode is its own (pdfimages), not a drawing.  Where a decoder
            disagrees (both draw some files blank), ours must equal the other one or SerenityOS's
            reference image.  The embedded form through codectest --jbig2-embedded must equal the
            file's first page.
  pdf       the same PDFs drawn by lib/pdf's renderer (pdftest --render: JBIG2Decode with its
            JBIG2Globals) must equal our decode, pixel for pixel
  codec     four files opened through img_all (codectest --convert-ex: found by their header,
            decoded by img_jbig2::CODEC) equal our decode
  mutate    damaged copies of four files (codectest --jbig2-mutate): no crash, none slower
            than 2 s
  layout    files made here (codectest --jbig2): two pages whose segments interleave, two pages
            without an end of page, a global pattern dictionary after a damaged page

  python tools/jbig2_check.py [--exe build/linux/codectest] [--pdftest build/linux/pdftest] [--wine]
                               [--only SUITE] [--keep DIR]

Exit code 1 when a check fails.  A checksum over every decoded page closes the run (the same on
every target).
"""
import argparse
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from suite_fetch import fetch  # noqa: E402
from pdf_read_check import Pdf  # noqa: E402

try:
    from PIL import Image
except ImportError:
    Image = None

SERENITY = ("SerenityOS/serenity", "835d22663febc14427577fc91468c1e4c1fcc914")
SER_DIR = "Tests/LibGfx/test-inputs/jbig2/"
FILES = """annex-h bitmap-composite-and-xnor-halftone bitmap-composite-and-xnor-refine bitmap-composite-and-xnor-text
bitmap-composite-and-xnor bitmap-composite-or-xor-replace-halftone bitmap-composite-or-xor-replace-refine
bitmap-composite-or-xor-replace-text bitmap-composite-or-xor-replace bitmap-customat-tpgdon bitmap-customat
bitmap-halftone-10bpp-mmr bitmap-halftone-10bpp bitmap-halftone-composite bitmap-halftone-global bitmap-halftone-grid
bitmap-halftone-refine bitmap-halftone-skip-dummy bitmap-halftone-skip-grid-template1 bitmap-halftone-skip-grid-template2
bitmap-halftone-skip-grid-template3 bitmap-halftone-skip-grid bitmap-halftone-template1 bitmap-halftone-template2
bitmap-halftone-template3 bitmap-halftone bitmap-initially-unknown-size bitmap-mmr bitmap-p32-eof bitmap-randomaccess
bitmap-refine-customat-tpgron bitmap-refine-customat bitmap-refine-lossless bitmap-refine-page-subrect bitmap-refine-page
bitmap-refine-refine bitmap-refine-template1-tpgron bitmap-refine-template1 bitmap-refine-tpgron bitmap-refine
bitmap-stripe-initially-unknown-height bitmap-stripe-last-implicit bitmap-stripe-single-no-end-of-stripe bitmap-stripe-single
bitmap-stripe bitmap-symbol-32bit-arithint bitmap-symbol-big-segmentid bitmap-symbol-context-reuse-huffman-refagg
bitmap-symbol-context-reuse-refagg bitmap-symbol-context-reuse bitmap-symbol-empty bitmap-symbol-global
bitmap-symbol-manyrefs bitmap-symbol-manysyms bitmap-symbol-negative-sbdsoffset bitmap-symbol-refine
bitmap-symbol-symbolrefine-textrefine-export bitmap-symbol-symbolrefine-textrefine bitmap-symbol-symbolrefineone-customat
bitmap-symbol-symbolrefineone-template1 bitmap-symbol-symbolrefineone bitmap-symbol-symbolrefineseveral
bitmap-symbol-symhuff-texthuff-manysyms bitmap-symbol-symhuff-texthuff bitmap-symbol-symhuff-texthuffB10B13
bitmap-symbol-symhuffB5B3-texthuffB7B9B12 bitmap-symbol-symhuffcustom-texthuffcustom
bitmap-symbol-symhuffrefine-textrefine-export bitmap-symbol-symhuffrefine-textrefine bitmap-symbol-symhuffrefineone
bitmap-symbol-symhuffrefineseveral bitmap-symbol-symhuffuncompressed-texthuff bitmap-symbol-textbottomleft
bitmap-symbol-textbottomlefttranspose bitmap-symbol-textbottomright bitmap-symbol-textbottomrighttranspose
bitmap-symbol-textcomposite bitmap-symbol-texthuff-runcodes32-34 bitmap-symbol-texthuff-trailingsymbols
bitmap-symbol-texthuffrefine bitmap-symbol-texthuffrefineB15 bitmap-symbol-texthuffrefinecustom
bitmap-symbol-texthuffrefinecustomdims bitmap-symbol-texthuffrefinecustompos-global bitmap-symbol-texthuffrefinecustompos
bitmap-symbol-texthuffrefinecustomposdims bitmap-symbol-texthuffrefinecustomsize bitmap-symbol-textrefine-customat
bitmap-symbol-textrefine-negative-delta-width bitmap-symbol-textrefine bitmap-symbol-texttopright
bitmap-symbol-texttoprighttranspose bitmap-symbol-texttranspose bitmap-symbol bitmap-template1-customat-tpgdon
bitmap-template1-customat bitmap-template1-tpgdon bitmap-template1 bitmap-template2-customat-tpgdon bitmap-template2-customat
bitmap-template2-tpgdon bitmap-template2 bitmap-template3-customat-tpgdon bitmap-template3-customat bitmap-template3-tpgdon
bitmap-template3 bitmap-tpgdon bitmap-trailing-7fff-stripped-harder-refine bitmap-trailing-7fff-stripped-harder
bitmap-trailing-7fff-stripped bitmap""".split()
# the reference image of a file, where Ghostscript or Poppler (or both) draw it otherwise: these files
# are encodings of SerenityOS's bitmap.bmp (json/bitmap.bmp, the image its tests decode them to);
# only a file's own reference image counts
REF_OF = dict.fromkeys("""bitmap-composite-and-xnor-halftone bitmap-composite-and-xnor-refine
bitmap-composite-or-xor-replace-refine bitmap-halftone-composite bitmap-halftone-refine
bitmap-halftone-skip-dummy bitmap-halftone-skip-grid bitmap-halftone-skip-grid-template1
bitmap-halftone-skip-grid-template2 bitmap-halftone-skip-grid-template3 bitmap-refine
bitmap-refine-customat bitmap-refine-customat-tpgron bitmap-refine-lossless bitmap-refine-page-subrect
bitmap-refine-refine bitmap-refine-template1 bitmap-refine-template1-tpgron bitmap-refine-tpgron
bitmap-symbol-context-reuse bitmap-symbol-context-reuse-huffman-refagg
bitmap-symbol-context-reuse-refagg bitmap-symbol-manyrefs bitmap-symbol-symhuff-texthuff
bitmap-symbol-symhuff-texthuffB10B13 bitmap-symbol-symhuffB5B3-texthuffB7B9B12
bitmap-symbol-symhuffcustom-texthuffcustom bitmap-symbol-symhuffrefine-textrefine
bitmap-symbol-symhuffrefine-textrefine-export bitmap-symbol-symhuffrefineone
bitmap-symbol-symhuffrefineseveral bitmap-symbol-texthuffrefine bitmap-symbol-texthuffrefineB15
bitmap-symbol-texthuffrefinecustom bitmap-symbol-texthuffrefinecustomdims
bitmap-symbol-texthuffrefinecustompos bitmap-symbol-texthuffrefinecustompos-global
bitmap-symbol-texthuffrefinecustomposdims bitmap-symbol-texthuffrefinecustomsize
bitmap-trailing-7fff-stripped-harder-refine""".split(), "bitmap.bmp")
REFS = sorted(set(REF_OF.values()))

FAILS = []
DIGEST = hashlib.sha256()


def fail(msg):
    FAILS.append(msg)
    print("FAIL " + msg)


def read_pbm(p):
    b = Path(p).read_bytes()
    parts = b.split(b"\n", 2)
    w, h = map(int, parts[1].split())
    stride = (w + 7) // 8
    return w, h, parts[2][: stride * h]


def rows_of_image(img):
    """a PIL image as (w, h, bits): 1 = black, MSB first, rows padded to bytes"""
    img = img.convert("1")
    w, h = img.size
    stride = (w + 7) // 8
    px = img.load()
    out = bytearray(stride * h)
    for y in range(h):
        for x in range(w):
            if px[x, y] == 0:
                out[y * stride + x // 8] |= 0x80 >> (x % 8)
    return w, h, bytes(out)


# ------------------------------------------------------------------ JBIG2 segments (7.2)

def segments(d):
    """the segments of a JBIG2 file: (header bytes, data bytes, number, type, page)"""
    assert d[:8] == b"\x97JB2\r\n\x1a\n"
    flags = d[8]
    p = 9 if flags & 2 else 13
    heads = []

    def header(p):
        start = p
        num, = struct.unpack_from(">I", d, p)
        sflags, rts = d[p + 4], d[p + 5]
        p += 6
        count = rts >> 5
        if count == 7:
            count = struct.unpack_from(">I", d, p - 1)[0] & 0x1FFFFFFF
            p += 3 + (count + 1 + 7) // 8
        size = 1 if num <= 256 else 2 if num <= 65536 else 4
        p += count * size
        if sflags & 0x40:
            page, = struct.unpack_from(">I", d, p)
            p += 4
        else:
            page = d[p]
            p += 1
        length, = struct.unpack_from(">I", d, p)
        p += 4
        return start, p, num, sflags & 0x3F, page, length

    out = []
    if flags & 1:
        while p < len(d):
            start, p, num, ty, page, length = header(p)
            if length == 0xFFFFFFFF:
                mmr = d[p + 17] & 1
                marker = b"\x00\x00" if mmr else b"\xff\xac"
                i = d.index(marker, p + 18)
                length = i + 6 - p
            out.append((d[start:p], d[p:p + length], num, ty, page))
            p += length
            if ty == 51:
                break
    else:
        while True:
            start, p, num, ty, page, length = header(p)
            heads.append((d[start:p], num, ty, page, length))
            if ty == 51 or p >= len(d):
                break
        for h, num, ty, page, length in heads:
            out.append((h, d[p:p + length], num, ty, page))
            p += length
    return out


def embedded(d):
    """PDF's form of a file's first page: (the page's segments, the global ones)"""
    segs = segments(d)
    first = next((s[4] for s in segs if s[3] == 48), 1)
    page = b"".join(h + data for h, data, num, ty, pg in segs if pg == first and ty not in (51,))
    glob = b"".join(h + data for h, data, num, ty, pg in segs if pg == 0 and ty not in (51,))
    return page, glob


def pdf_of(page_data, glob, w, h):
    p = Pdf()
    p.add(b"", 1)
    p.add(b"", 2)
    extra = b""
    if glob:
        g = p.add(Pdf.stream(b"", glob))
        extra = b" /DecodeParms << /JBIG2Globals %d 0 R >>" % g
    im = p.add(Pdf.stream(b"/Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode" % (w, h) + extra, page_data, compress=False))
    c = p.add(Pdf.stream(b"", b"q %d 0 0 %d 0 0 cm /I Do Q" % (w, h)))
    pg = p.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %d %d] /Contents %d 0 R /Resources << /XObject << /I %d 0 R >> >> >>" % (w, h, c, im))
    p.objs[2] = b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % pg
    p.objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
    return p.write(1)


def render_gs(pdf, work):
    out = Path(work) / "gs.png"
    r = subprocess.run(["gs", "-q", "-dSAFER", "-dNOPAUSE", "-dBATCH", "-sDEVICE=pngmono", "-r72", "-dGraphicsAlphaBits=1", "-dInterpolateControl=0", "-sOutputFile=%s" % out, str(pdf)], capture_output=True, timeout=120)
    if r.returncode != 0 or not out.exists():
        return None
    return rows_of_image(Image.open(out))


def render_poppler(pdf, work):
    """Poppler's decode of the image itself (pdfimages), not a drawing of it"""
    base = Path(work) / "pp"
    for old in Path(work).glob("pp-*"):
        old.unlink()
    r = subprocess.run(["pdfimages", "-png", str(pdf), str(base)], capture_output=True, timeout=120)
    p = Path(str(base) + "-000.png")
    if r.returncode != 0 or not p.exists():
        return None
    return rows_of_image(Image.open(p))


class Exe:
    def __init__(self, exe, wine):
        self.exe, self.wine = str(exe), wine

    def run(self, *args):
        c = [self.exe, *map(str, args)]
        return subprocess.run(["wine", *c] if self.wine else c, capture_output=True, text=True, timeout=300, env=dict(os.environ, WINEDEBUG="-all"))


def differ(a, b):
    """the pixels that differ between two (w, h, bits), or None when the sizes differ or the bits of
    one are short"""
    if a is None or b is None or a[:2] != b[:2]:
        return None
    n = (a[0] + 7) // 8 * a[1]
    if len(a[2]) != n or len(b[2]) != n:
        return None
    return sum(bin(x ^ y).count("1") for x, y in zip(a[2], b[2]))


def suite_serenity(exe, work):
    folder = fetch(*SERENITY, [SER_DIR + n + ".jbig2" for n in FILES] + [SER_DIR + "json/" + r for r in REFS] + ["LICENSE"])
    d = folder / SER_DIR
    refs = {}
    if Image:
        for r in REFS:
            refs[r] = rows_of_image(Image.open(d / "json" / r))
    n_ok = 0
    for name in FILES:
        path = d / (name + ".jbig2")
        out = Path(work) / "ours.pbm"
        r = exe.run("--jbig2", path, "--to", out)
        if r.returncode != 0:
            fail("%s: %s" % (name, r.stderr.strip()))
            continue
        ours = read_pbm(out)
        DIGEST.update(ours[2])
        k = 2
        while (Path(work) / ("ours.pbm-%d" % k)).exists():
            DIGEST.update(read_pbm(Path(work) / ("ours.pbm-%d" % k))[2])
            (Path(work) / ("ours.pbm-%d" % k)).unlink()
            k += 1
        # the embedded form
        raw = path.read_bytes()
        page, glob = embedded(raw)
        (Path(work) / "page.jb2").write_bytes(page)
        (Path(work) / "glob.jb2").write_bytes(glob)
        args = ["--jbig2-embedded", Path(work) / "page.jb2", "--to", Path(work) / "emb.pbm"] + (["--globals", Path(work) / "glob.jb2"] if glob else [])
        r = exe.run(*args)
        if r.returncode != 0 or read_pbm(Path(work) / "emb.pbm") != ours:
            fail("%s: the embedded form decodes differently (%s)" % (name, r.stderr.strip()))
        if Image is None:
            continue
        # jbig2dec and Poppler
        pdf = Path(work) / "t.pdf"
        pdf.write_bytes(pdf_of(page, glob, ours[0], ours[1]))
        g = render_gs(pdf, work)
        pp = render_poppler(pdf, work)
        dg, dp = differ(ours, g), differ(ours, pp)
        if dg == 0 and dp == 0:
            n_ok += 1
            continue
        # where a decoder disagrees: ours must equal the other one or SerenityOS's reference image
        votes = [REF_OF[name]] if name in REF_OF and differ(ours, refs.get(REF_OF[name])) == 0 else []
        agree = [n for n, dd in (("Ghostscript", dg), ("Poppler", dp)) if dd == 0] + votes
        if agree:
            n_ok += 1
            print("%s: ours = %s; Ghostscript differs by %s pixels, Poppler by %s" % (name, " = ".join(agree), dg, dp))
            continue
        fail("%s: %sx%s, %s pixels differ from Ghostscript, %s from Poppler, none of the reference images" % (name, ours[0], ours[1], dg, dp))
    print("serenity: %d of %d files decoded as jbig2dec, Poppler or SerenityOS's reference decode them" % (n_ok, len(FILES)))


def read_ppm_mono(p):
    """a binary PPM as (w, h, bits): 1 where the pixel is dark"""
    b = Path(p).read_bytes()
    parts = b.split(b"\n", 3)
    w, h = map(int, parts[1].split())
    px = parts[3]
    stride = (w + 7) // 8
    out = bytearray(stride * h)
    for y in range(h):
        for x in range(w):
            if px[(y * w + x) * 3] < 128:
                out[y * stride + x // 8] |= 0x80 >> (x % 8)
    return w, h, bytes(out)


def suite_pdf(exe, work, pdftest):
    """lib/pdf's renderer (JBIG2Decode with JBIG2Globals, the page drawn at 72 dpi) against our decode
    of the file"""
    folder = fetch(*SERENITY, [SER_DIR + n + ".jbig2" for n in FILES])
    d = folder / SER_DIR
    n_ok = 0
    for name in FILES:
        path = d / (name + ".jbig2")
        out = Path(work) / "ours.pbm"
        if exe.run("--jbig2", path, "--to", out).returncode != 0:
            fail("pdf %s: not decoded" % name)
            continue
        ours = read_pbm(out)
        page, glob = embedded(path.read_bytes())
        pdf = Path(work) / "t.pdf"
        pdf.write_bytes(pdf_of(page, glob, ours[0], ours[1]))
        for old in Path(work).glob("r-*.ppm"):
            old.unlink()
        r = subprocess.run([str(pdftest), "--render", str(pdf), "--out", str(Path(work) / "r")], capture_output=True, timeout=300)
        got = Path(work) / "r-1.ppm"
        if r.returncode != 0 or not got.exists():
            fail("pdf %s: pdftest --render failed" % name)
            continue
        dd = differ(ours, read_ppm_mono(got))
        if dd == 0:
            n_ok += 1
        else:
            fail("pdf %s: the page drawn by lib/pdf differs from the decode in %s pixels" % (name, dd))
    print("pdf: %d of %d files drawn by lib/pdf as decoded" % (n_ok, len(FILES)))


def suite_codec(exe, work):
    """img_all's registration: codectest --convert-ex finds a JBIG2 file by its probe and decodes its
    first page through img_jbig2::CODEC into a PNG"""
    if Image is None:
        print("codec: skipped (no Pillow)")
        return
    folder = fetch(*SERENITY, [SER_DIR + n + ".jbig2" for n in FILES])
    d = folder / SER_DIR
    names = ["bitmap", "bitmap-symbol-symhuff-texthuff", "bitmap-halftone", "annex-h"]
    for name in names:
        png = Path(work) / "codec.png"
        r = exe.run("--convert-ex", d / (name + ".jbig2"), "--to", png)
        if r.returncode != 0 or not png.exists():
            fail("codec %s: --convert-ex exit %d" % (name, r.returncode))
            continue
        exe.run("--jbig2", d / (name + ".jbig2"), "--to", Path(work) / "ours.pbm")
        dd = differ(rows_of_image(Image.open(png)), read_pbm(Path(work) / "ours.pbm"))
        if dd != 0:
            fail("codec %s: img_all's page differs in %s pixels" % (name, dd))
    print("codec: %d files through img_all" % len(names))


def suite_mutate(exe, work):
    folder = fetch(*SERENITY, [SER_DIR + n + ".jbig2" for n in FILES])
    d = folder / SER_DIR
    for name in ("bitmap-symbol-symbolrefine-textrefine", "bitmap-symbol-symhuffrefine-textrefine", "bitmap-halftone-10bpp-mmr", "annex-h"):
        r = exe.run("--jbig2-mutate", d / (name + ".jbig2"), "--count", 2000, "--seed", 5)
        line = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else ""
        if r.returncode != 0 or "slowest" not in line:
            fail("mutate %s: exit %d %s" % (name, r.returncode, (r.stderr or "")[-200:]))
            continue
        print("mutate %s" % line.split(": ", 1)[1].replace(str(d) + "/", ""))
        if int(line.split("slowest ")[1].split(" ")[0]) > 2000:
            fail("mutate %s: a damaged copy took too long" % name)


JB2_ID = bytes([0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A])


def jb2_seg(num, ty, page, data, refs=()):
    """a segment (T.88 7.2): its number, type, referred-to segments (numbers below 256), page
    association (one byte) and data"""
    return struct.pack(">IBB", num, ty, len(refs) << 5) + bytes(refs) + bytes([page]) + struct.pack(">I", len(data)) + data


def suite_layout(exe, work):
    """files made here, decoded as files: the segments of two pages interleaved (a region goes to
    the page its association names), two pages without an end of page, and a global pattern
    dictionary after a damaged page (the next page's halftone region still finds it)"""
    info = struct.pack(">IIII", 8, 2, 0, 0) + b"\x00\x00\x00"
    right = (8, 2, b"\x0f\x0f")  # the right half black
    white = (8, 2, b"\x00\x00")
    # an MMR generic region of 8 x 2 at (0, 0), OR: four white and four black in both rows
    region = struct.pack(">IIII", 8, 2, 0, 0) + b"\x00\x01" + b"\x36\xf0"
    # a pattern dictionary (MMR): two patterns of 4 x 2, white and black
    patterns = b"\x01" + bytes([4, 2]) + struct.pack(">I", 1) + b"\x36\xf0"
    # a halftone region (MMR) of 8 x 2: a grid of two cells 4 pixels apart, grey values 0 and 1
    halftone = struct.pack(">IIII", 8, 2, 0, 0) + b"\x00" + b"\x01" + struct.pack(">IIiiHH", 2, 1, 0, 0, 4 * 256, 0) + b"\x23\xa0"
    cases = {
        "interleaved": ([jb2_seg(0, 48, 1, info), jb2_seg(1, 48, 2, info), jb2_seg(2, 39, 1, region), jb2_seg(3, 49, 1, b""), jb2_seg(4, 49, 2, b""), jb2_seg(5, 51, 0, b"")], [right, white]),
        "no-end-of-page": ([jb2_seg(0, 48, 1, info), jb2_seg(1, 39, 1, region), jb2_seg(2, 48, 2, info)], [right, white]),
        "global-after-damage": ([jb2_seg(0, 48, 1, info), jb2_seg(1, 38, 1, b"\x00" * 5), jb2_seg(2, 16, 0, patterns), jb2_seg(3, 49, 1, b""), jb2_seg(4, 48, 2, info), jb2_seg(5, 23, 2, halftone, [2]), jb2_seg(6, 49, 2, b"")], [white, right]),
    }
    for name, (segs, want) in cases.items():
        path = Path(work) / ("layout-%s.jb2" % name)
        path.write_bytes(JB2_ID + b"\x01" + struct.pack(">I", len(want)) + b"".join(segs))
        for old in Path(work).glob("layout.pbm*"):
            old.unlink()
        out = Path(work) / "layout.pbm"
        r = exe.run("--jbig2", path, "--to", out)
        if r.returncode != 0:
            fail("layout %s: %s" % (name, r.stderr.strip()))
            continue
        more = [Path(work) / ("layout.pbm-%d" % k) for k in range(2, 10)]
        got = [read_pbm(out)] + [read_pbm(m) for m in more if m.exists()]
        if got != want:
            fail("layout %s: the pages %s, not %s" % (name, got, want))
    print("layout: %d files made here" % len(cases))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "build" / "linux" / "codectest"))
    ap.add_argument("--wine", action="store_true")
    ap.add_argument("--only")
    ap.add_argument("--keep")
    ap.add_argument("--pdftest", default=str(ROOT / "build" / "linux" / "pdftest"))
    a = ap.parse_args()
    exe = Exe(a.exe, a.wine)
    work = a.keep or tempfile.mkdtemp(prefix="jbig2_check_")
    os.makedirs(work, exist_ok=True)
    suites = {"serenity": suite_serenity, "pdf": lambda e, w: suite_pdf(e, w, a.pdftest), "codec": suite_codec, "mutate": suite_mutate, "layout": suite_layout}
    for name, f in suites.items():
        if a.only and a.only != name:
            continue
        if name == "pdf" and (a.wine or not Path(a.pdftest).exists()):
            print("pdf: skipped (no %s)" % a.pdftest)
            continue
        f(exe, work)
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    print("checksum %s" % DIGEST.hexdigest()[:16])
    print("%d failures" % len(FAILS))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
