#!/usr/bin/env python3
"""reader_check.py -- lib/pdf's sample document checked by readers that are not ours.

Writes the sample with `pdftest --sample` and hands it to whatever is installed (nothing is
downloaded); a reader that is missing is reported and skipped:
  Poppler    pdfinfo (pages, metadata), pdffonts (every font embedded, subset, with ToUnicode),
             pdftotext (the text of every script on the pages), pdftoppm (renders every page)
  pypdf      strict parsing with warnings as errors, the outline tree, the links, the text
  pypdfium2  (pdfium, Chrome's reader) renders every page and finds the text

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
        "ABAB", "gedrehter Text", "100 × 100 mm", "https://github.com/JFLibrary/JFLib"]
ARABIC = ["السلام عليكم", "مرحبا"]
OUTLINE = ["1 Vektorgrafik", "1.1 Füllregeln", "1.2 Konturen", "1.3 Kurven und Transformationen",
           "1.4 Clipping und Transparenz", "2 Bilder, Text und Links", "2.1 Bilder", "2.2 Text",
           "Arabisch und Ersatzschrift", "3 Querformat (Letter, y nach oben)", "4 Kleine Seite (100 × 100 mm)"]

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
    if info.returncode or "Pages:           4" not in text or "PDF version:     1.7" not in text:
        fail(f"pdfinfo: {info.stderr.decode(errors='replace')}{text}")
    else:
        ok("pdfinfo: 4 pages, PDF 1.7")
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
    if r.returncode or len(pngs) != 4 or r.stderr:
        fail(f"pdftoppm: {r.stderr.decode(errors='replace')}")
    else:
        ok("pdftoppm: 4 pages rendered without a message")


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
        ok("pypdf: strict, outline of 11 items in order")
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
    if len(doc) != 4 or blank:
        fail(f"pdfium: {len(doc)} pages, blank: {blank}")
    else:
        ok("pdfium: 4 pages rendered")
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
