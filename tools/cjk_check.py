#!/usr/bin/env python3
"""cjk_check.py -- lib/pdf's Chinese, Japanese and Korean text against Python's codecs and Poppler

For each predefined CMap of ISO 32000-1 Table 118 (lib/cidmap) a PDF made here: a line of text
in the CMap's encoding (encoded by Python's codecs: gb2312, gbk, gb18030, big5, cp950, big5hkscs,
shift_jis, cp932, euc_jp, iso2022_jp for H and V, euc_kr, cp949, utf-16-be), in a CIDFontType0
that is not embedded, without a ToUnicode -- the text comes from the CMap and the collection's
CID -> Unicode table alone (9.10.2). Then:
  - `pdftest --read` must give the line (whitespace aside)
  - `pdftotext -raw` (Poppler with poppler-data) is compared and reported
  - the page drawn by `pdftest --render` and by `pdftoppm` (both with the system's font for the
    language, Noto Sans CJK here) must agree: of the pixels either one inks, at most --max %
    may differ (tools/pdf_render_check.py's measure)
CNS-EUC has no Python codec and is left out (cidmap_gen.py --check covers its data).

  python tools/cjk_check.py [--exe build/linux/pdftest] [--only NAME] [--keep DIR] [--max 5]

Exit code 1 when a check fails.
"""
import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
try:
    from PIL import Image, ImageChops
    from pdf_render_check import diff_metric
except ImportError:
    Image = None

TEXT = {
    "Japan1": "日本語のテキスト、漢字とかな。ABC 123",
    "GB1": "中文文本，汉字测试。ABC 123",
    "CNS1": "中文文本，漢字測試。ABC 123",
    "Korea1": "한국어 텍스트, 한글 시험. ABC 123",
}
# CMap -> (collection, codec)
CMAPS = {}
for names, coll, codec in [
    ("GB-EUC GBpc-EUC", "GB1", "gb2312"),
    ("GBK-EUC GBKp-EUC", "GB1", "gbk"),
    ("GBK2K", "GB1", "gb18030"),
    ("UniGB-UCS2 UniGB-UTF16", "GB1", "utf-16-be"),
    ("B5pc ETen-B5", "CNS1", "big5"),
    ("HKscs-B5", "CNS1", "big5hkscs"),
    ("ETenms-B5", "CNS1", "cp950"),
    ("UniCNS-UCS2 UniCNS-UTF16", "CNS1", "utf-16-be"),
    ("90ms-RKSJ 90msp-RKSJ Ext-RKSJ", "Japan1", "cp932"),
    ("83pv-RKSJ 90pv-RKSJ Add-RKSJ", "Japan1", "shift_jis"),
    ("EUC", "Japan1", "euc_jp"),
    ("", "Japan1", "jis"),
    ("UniJIS-UCS2 UniJIS-UCS2-HW UniJIS-UTF16", "Japan1", "utf-16-be"),
    ("KSC-EUC KSCpc-EUC", "Korea1", "euc_kr"),
    ("KSCms-UHC KSCms-UHC-HW", "Korea1", "cp949"),
    ("UniKS-UCS2 UniKS-UTF16", "Korea1", "utf-16-be"),
]:
    for n in names.split() or ["H"]:
        pre = "" if n == "H" else n + "-"
        for wm in ("H", "V"):
            CMAPS[pre + wm if pre else wm] = (coll, codec)
# the ones Table 118 lists only horizontal
for n in ("83pv-RKSJ-V", "90pv-RKSJ-V", "KSCpc-EUC-V"):
    del CMAPS[n]


def encode(text, codec):
    if codec == "jis":
        # JIS X 0208 in seven bits without the escapes (the CMaps H and V): no ASCII here
        b = text.encode("iso2022_jp")
        return re.sub(rb"\x1b\([BJ]|\x1b\$[@B]", b"", b)
    return text.encode(codec)


def sample(coll, codec):
    t = TEXT[coll]
    if codec == "jis":
        t = t.split("ABC")[0]
    return t


def pdf(name, coll, data, vertical):
    """one page: the line in a CIDFontType0 of the collection that is not embedded"""
    content = (b"BT /F1 24 Tf %s Td <%s> Tj ET" % (b"280 760" if vertical else b"30 400", data.hex().encode()))
    content = zlib.compress(content)
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(content) + content + b"\nendstream",
        b"<< /Type /Font /Subtype /Type0 /BaseFont /CJKTest /Encoding /%s /DescendantFonts [6 0 R] >>" % name.encode(),
        b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /CJKTest /CIDSystemInfo << /Registry (Adobe) /Ordering (%s) /Supplement 2 >> /FontDescriptor 7 0 R /DW 1000 >>" % coll.encode(),
        b"<< /Type /FontDescriptor /FontName /CJKTest /Flags 4 /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>",
    ]
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offs = []
    for i, o in enumerate(objs):
        offs.append(len(out))
        out += b"%d 0 obj\n" % (i + 1) + o + b"\nendobj\n"
    x = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    for o in offs:
        out += b"%010d 00000 n \n" % o
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, x)
    return bytes(out)


def vertical(name):
    return name == "V" or name.endswith("-V")


def squash(s):
    return "".join(s.split())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "build" / "linux" / "pdftest"))
    ap.add_argument("--only")
    ap.add_argument("--keep")
    ap.add_argument("--max", type=float, default=5.0, help="the largest share (in %%) of the inked pixels that may differ from pdftoppm")
    a = ap.parse_args()
    work = Path(a.keep) if a.keep else Path(tempfile.mkdtemp(prefix="cjk_check_"))
    work.mkdir(parents=True, exist_ok=True)
    fails = []
    ok = poppler_ok = drawn_ok = 0
    names = [n for n in CMAPS if not a.only or n == a.only]
    for name in names:
        coll, codec = CMAPS[name]
        want = sample(coll, codec)
        f = work / (name + ".pdf")
        f.write_bytes(pdf(name, coll, encode(want, codec), vertical(name)))
        # our text
        r = subprocess.run([a.exe, "--read", str(f)], capture_output=True, text=True, timeout=120)
        try:
            ours = "".join(json.loads(r.stdout)["text"])
        except (ValueError, KeyError):
            ours = None
        if ours is not None and squash(ours) == squash(want):
            ok += 1
        else:
            fails.append("%s: text %r, not %r" % (name, ours, want))
            print("FAIL " + fails[-1])
        # Poppler's text
        p = subprocess.run(["pdftotext", "-raw", "-enc", "UTF-8", str(f), "-"], capture_output=True, timeout=120)
        theirs = p.stdout.decode("utf-8", "replace")
        if squash(theirs) == squash(want):
            poppler_ok += 1
        else:
            print("  %s: Poppler gives %r" % (name, theirs.strip()))
        # drawn
        if Image is None or not shutil.which("pdftoppm"):
            continue
        for q in work.glob(name + "-*.p?m"):
            q.unlink()
        subprocess.run([a.exe, "--render", str(f), "--out", str(work / (name + "-ours"))], capture_output=True, timeout=300)
        subprocess.run(["pdftoppm", "-r", "72", str(f), str(work / (name + "-ref"))], capture_output=True, timeout=300)
        mine = sorted(work.glob(name + "-ours*.ppm"))
        ref = sorted(work.glob(name + "-ref*.ppm"))
        if not mine or not ref:
            fails.append("%s: not drawn (ours %d, pdftoppm %d)" % (name, len(mine), len(ref)))
            print("FAIL " + fails[-1])
            continue
        ia, ib = Image.open(mine[0]).convert("RGB"), Image.open(ref[0]).convert("RGB")
        if ia.size != ib.size:
            fails.append("%s: drawn %s, pdftoppm %s" % (name, ia.size, ib.size))
            print("FAIL " + fails[-1])
            continue
        bad, mean, _ = diff_metric(ia, ib, 48)
        dark = lambda im: im.convert("L").point(lambda v: 255 if v < 128 else 0)
        ink = dark(ia).histogram()[255]
        both = ImageChops.lighter(dark(ia), dark(ib)).histogram()[255]
        share = bad * ia.width * ia.height / max(both, 1) * 100
        if share > a.max or ink == 0:
            fails.append("%s: %.1f %% of the inked pixels differ from pdftoppm (ours %d, both %d)" % (name, share, ink, both))
            print("FAIL " + fails[-1])
        else:
            drawn_ok += 1
    print("cjk_check: %d of %d CMaps give their text, Poppler %d; %d drawn as pdftoppm draws them" % (ok, len(names), poppler_ok, drawn_ok))
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    print("%d failures" % len(fails))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
