#!/usr/bin/env python3
"""reader_check.py -- lib/pdf's sample document checked by readers that are not ours.

Writes the sample with `pdftest --sample` and hands it to whatever is installed (nothing is
downloaded); a reader that is missing is reported and skipped:
  Poppler    pdfinfo (pages, metadata), pdffonts (every font embedded, subset, with ToUnicode),
             pdftotext (the text of every script on the pages), pdftoppm (renders every page;
             pixels of the gradients and hatches on page 5)
  pypdf      strict parsing with warnings as errors, the outline tree, the links, the text
  pypdfium2  (pdfium, Chrome's reader) renders every page, finds the text, the same pixels

  python apps/pdftest/reader_check.py [--exe build/win/pdftest.exe] [--keep DIR]

Exit code 1 when a reader that is there finds a problem.  pdfium returns right-to-left words in
visual order and reverses the letters a ligature glyph stands for (lam-alef); its Arabic is only
checked word by word without the ligature.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

TEXT = ["JFLib PDF – Vektor-PDF-Schreiber", "Größe, Übermaß, Ärger", "„Anführungen“", "3 × 4 ≥ 10 €",
        "Καλημέρα κόσμε", "Здравствуй, мир", "Pfiff", "office", "AVATAR", "Unsichtbarer Text", "AVOBFGJQ",
        "ABAB", "gedrehter Text", "100 × 100 mm", "https://github.com/JFLibrary/JFLib",
        "Verläufe und Schraffuren", "Radiale Verläufe"]
ARABIC = ["السلام عليكم", "مرحبا"]
OUTLINE = ["1 Vektorgrafik", "1.1 Füllregeln", "1.2 Konturen", "1.3 Kurven und Transformationen",
           "1.4 Clipping und Transparenz", "2 Bilder, Text und Links", "2.1 Bilder", "2.2 Text",
           "Arabisch und Ersatzschrift", "3 Querformat (Letter, y nach oben)", "4 Kleine Seite (100 × 100 mm)",
           "5 Verläufe und Schraffuren"]
PAGES = 5

# page 5 at 72 dpi (one pixel per point, sample.rs's top-left coordinates): (x, y), the colour
# expected, the tolerance per channel, what it shows.  The values follow from the geometry in
# sample.rs::gradients_page (pixel centres at x + 0.5).
PIXELS = [
    ((52, 145), (252, 0, 3), 14, "axial red to blue: start"),
    ((150, 145), (127, 0, 128), 14, "axial red to blue: middle"),
    ((248, 145), (2, 0, 253), 14, "axial red to blue: end"),
    ((416, 145), (240, 131, 2), 16, "four stops: orange left of the hard edge"),
    ((424, 145), (31, 157, 63), 16, "four stops: green right of the hard edge"),
    ((52, 260), (35, 82, 193), 14, "soft mask: alpha 1 at the start"),
    ((150, 260), (144, 168, 224), 14, "soft mask: alpha 0.5 in the middle"),
    ((248, 260), (253, 254, 255), 10, "soft mask: alpha 0 at the end"),
    ((120, 510), (252, 252, 253), 12, "radial: white centre"),
    ((176, 510), (30, 45, 105), 16, "radial: dark blue near radius 60"),
    ((212, 452), (64, 16, 16), 12, "radial extended to the corner"),
    ((460, 510), (48, 192, 240), 12, "shade() in the ellipse: start colour"),
    ((460, 450), (255, 255, 255), 4, "shade() clipped: outside the ellipse"),
    ((120, 640), (160, 32, 32), 14, "hatch: on a line"),
    ((120, 644), (255, 255, 255), 14, "hatch: in a gap"),
    ((120, 648), (160, 32, 32), 14, "hatch: the next line"),
    ((120, 652), (255, 255, 255), 14, "hatch: the next gap"),
]

problems = []


def fail(what):
    problems.append(what)
    print("FAIL", what)


def ok(what):
    print("ok  ", what)


def run(args):
    return subprocess.run(args, capture_output=True, check=False)


def poppler(pdf, out):
    if not shutil.which("pdfinfo"):
        print("skip Poppler: pdfinfo is not on PATH")
        return
    info = run(["pdfinfo", "-enc", "UTF-8", str(pdf)])
    text = info.stdout.decode("utf-8", "replace")
    if info.returncode or f"Pages:           {PAGES}" not in text or "PDF version:     1.7" not in text:
        fail(f"pdfinfo: {info.stderr.decode(errors='replace')}{text}")
    else:
        ok(f"pdfinfo: {PAGES} pages, PDF 1.7")
    if "Title:           JFLib PDF – Beispiel" in text and "Metadata Stream: yes" in text:
        ok("pdfinfo: title and XMP metadata")
    else:
        fail("pdfinfo: title or metadata missing")
    fonts = run(["pdffonts", str(pdf)])
    rows = [line.split() for line in fonts.stdout.decode(errors="replace").splitlines()[2:] if line.strip()]
    bad = [r for r in rows if r[-5:-2] != ["yes", "yes", "yes"]]
    if fonts.returncode or len(rows) != 5 or bad or fonts.stderr:
        fail(f"pdffonts: {fonts.stdout.decode(errors='replace')}{fonts.stderr.decode(errors='replace')}")
    else:
        ok("pdffonts: 5 fonts, all embedded, subset, with ToUnicode")
    t = run(["pdftotext", "-enc", "UTF-8", str(pdf), "-"])
    s = t.stdout.decode("utf-8", "replace")
    missing = [k for k in TEXT + ARABIC if k not in s]
    if t.returncode or missing or t.stderr:
        fail(f"pdftotext: missing {missing} {t.stderr.decode(errors='replace')}")
    else:
        ok("pdftotext: every text, Arabic in logical order")
    r = run(["pdftoppm", "-r", "50", "-png", str(pdf), str(out / "poppler")])
    pngs = sorted(out.glob("poppler-*.png"))
    if r.returncode or len(pngs) != PAGES or r.stderr:
        fail(f"pdftoppm: {r.stderr.decode(errors='replace')}")
    else:
        ok(f"pdftoppm: {PAGES} pages rendered without a message")
    r = run(["pdftoppm", "-r", "72", "-f", "5", "-l", "5", "-singlefile", str(pdf), str(out / "page5")])
    ppm = out / "page5.ppm"
    if r.returncode or r.stderr or not ppm.is_file():
        fail(f"pdftoppm page 5: {r.stderr.decode(errors='replace')}")
    else:
        d = ppm.read_bytes()
        head = d.split(maxsplit=4)
        w, h = int(head[1]), int(head[2])
        pix = head[4][-w * h * 3:]
        pixels("Poppler", lambda x, y: tuple(pix[(y * w + x) * 3:(y * w + x) * 3 + 3]))


def pixels(who, at):
    """PIXELS against a renderer's page 5: at(x, y) -> (r, g, b)"""
    bad = []
    for (x, y), want, tol, what in PIXELS:
        got = at(x, y)
        if any(abs(a - b) > tol for a, b in zip(got, want)):
            bad.append(f"{what} at {x},{y}: {got}, want {want} +-{tol}")
    if bad:
        fail(f"{who} pixels of page 5: " + "; ".join(bad))
    else:
        ok(f"{who}: {len(PIXELS)} pixels of the gradients and hatches on page 5")


def pypdf_check(pdf):
    try:
        import warnings
        import pypdf
    except ImportError:
        print("skip pypdf: not installed")
        return
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        try:
            r = pypdf.PdfReader(str(pdf), strict=True)
            titles = []

            def walk(items):
                for it in items:
                    if isinstance(it, list):
                        walk(it)
                    else:
                        titles.append(it.title)
                        r.get_destination_page_number(it)
            walk(r.outline)
            links = [a.get_object() for p in r.pages for a in (p.get("/Annots") or [])]
            text = "".join(p.extract_text() for p in r.pages)
        except Exception as e:  # pypdf raises its own errors and the warnings turned into errors
            fail(f"pypdf: {type(e).__name__}: {e}")
            return
    if titles != OUTLINE:
        fail(f"pypdf: outline {titles}")
    else:
        ok(f"pypdf: strict, outline of {len(OUTLINE)} items in order")
    uris = [a["/A"]["/URI"] for a in links if "/A" in a]
    if len(links) != 5 or uris != ["https://github.com/JFLibrary/JFLib"]:
        fail(f"pypdf: links {links}")
    else:
        ok("pypdf: 5 links, the URI")
    missing = [k for k in TEXT if k not in text]
    if missing:
        fail(f"pypdf text: missing {missing}")
    else:
        ok("pypdf: text")


def pdfium_check(pdf):
    try:
        import pypdfium2 as pdfium
    except ImportError:
        print("skip pdfium: pypdfium2 not installed")
        return
    doc = pdfium.PdfDocument(str(pdf))
    text = ""
    blank = []
    for i in range(len(doc)):
        page = doc[i]
        text += page.get_textpage().get_text_range() + "\n"
        bm = page.render(scale=0.5)
        buf = bytes(bm.buffer)
        if buf.count(255) > len(buf) * 0.995:
            blank.append(i + 1)
    if len(doc) != PAGES or blank:
        fail(f"pdfium: {len(doc)} pages, blank: {blank}")
    else:
        ok(f"pdfium: {PAGES} pages rendered")
    bm = doc[4].render(scale=1)
    buf, n, stride = bytes(bm.buffer), bm.n_channels, bm.stride
    # pdfium's bitmaps are BGR or BGRA
    pixels("pdfium", lambda x, y: (buf[y * stride + x * n + 2], buf[y * stride + x * n + 1], buf[y * stride + x * n]))
    words = ["عليكم", "مرحبا"]
    missing = [k for k in TEXT + words if k not in text]
    if missing:
        fail(f"pdfium text: missing {missing}")
    else:
        ok("pdfium: text (Arabic word by word)")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    exe = ROOT / "build" / "win" / ("pdftest.exe" if os.name == "nt" else "pdftest")
    ap.add_argument("--exe", default=str(exe))
    ap.add_argument("--keep", help="keep the sample and the renderings in this folder")
    opt = ap.parse_args()
    out = Path(opt.keep) if opt.keep else Path(tempfile.mkdtemp(prefix="pdftest-"))
    out.mkdir(parents=True, exist_ok=True)
    pdf = out / "sample.pdf"
    r = run([os.path.abspath(opt.exe), "--sample", str(pdf)])
    if r.returncode or not pdf.is_file():
        print(f"pdftest --sample failed: {r.stdout.decode(errors='replace')}{r.stderr.decode(errors='replace')}")
        return 1
    poppler(pdf, out)
    pypdf_check(pdf)
    pdfium_check(pdf)
    if not opt.keep:
        shutil.rmtree(out, ignore_errors=True)
    print(f"reader_check: {len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main())
