#!/usr/bin/env python3
"""pdf_read_check.py -- lib/pdf's reader (pdf::read) against Poppler on many kinds of PDF files

Builds a corpus in a temporary folder, from what is installed (nothing is downloaded):
  - the sample of lib/pdf's writer (pdftest --sample)
  - PDFs made here: classic tables, cross-reference and object streams, incremental updates,
    hybrid files, damaged tables, a cut file, simple and composite fonts, forms, attachments,
    page labels, outlines with named destinations
  - the encrypted files of apps/pdftest/tests/read (made by --fixtures: RC4 40 and 128 bit, AES
    128 and 256 bit, revisions 2 to 6), opened with the user and the owner password
  - Ghostscript: PostScript in the standard fonts distilled at PDF 1.4 and 1.7 with object
    streams, and every file of the corpus distilled again (Type 1C and CID fonts)
  - LibreOffice: an HTML page with headings, a table, links, form fields and several scripts
  - Poppler's pdfunite of several files
  - the PDFs of the system (/usr/share)
and reads each with `pdftest --read` and with Poppler: pdfinfo (pages, page sizes, title,
author, encryption), pdftotext -raw (the text of each page: the words must agree to 0.97 by
default), pdfdetach -list (attachments).

  python tools/pdf_read_check.py [--exe build/linux/pdftest] [--keep DIR] [--min 0.97] [-v]
  python tools/pdf_read_check.py --fixtures     write apps/pdftest/tests/read/*.pdf again

Exit code 1 when a file disagrees.
"""
import argparse
import collections
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unicodedata
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "apps/pdftest/tests/read"

# ------------------------------------------------------------------ a small PDF writer


class Pdf:
    """objects as bytes (without "n 0 obj"); write() lays them out with a table or streams"""

    def __init__(self):
        self.objs = {}

    def add(self, body, num=None):
        num = num or (max(self.objs, default=0) + 1)
        self.objs[num] = body
        return num

    @staticmethod
    def stream(dict_body, data, compress=True):
        if compress:
            data = zlib.compress(data)
            dict_body = b"/Filter /FlateDecode " + dict_body
        return b"<< " + dict_body + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"

    def write(self, root, info=None, version=b"1.7", trailer_extra=b"", xref_stream=False, objstm=False):
        out = bytearray(b"%PDF-" + version + b"\n%\xe2\xe3\xcf\xd3\n")
        offsets = {}
        nums = sorted(self.objs)
        in_stm = {}
        if objstm:
            # every object that is no stream goes into one object stream
            small = [n for n in nums if b"stream\n" not in self.objs[n]]
            head, body = [], bytearray()
            for i, n in enumerate(small):
                head.append(b"%d %d" % (n, len(body)))
                body += self.objs[n] + b"\n"
                in_stm[n] = i
            hb = b" ".join(head) + b"\n"
            stm_num = max(nums) + 1
            self.objs[stm_num] = Pdf.stream(b"/Type /ObjStm /N %d /First %d" % (len(small), len(hb)), hb + bytes(body))
            nums = sorted(self.objs)
        for n in nums:
            if n in in_stm:
                continue
            offsets[n] = len(out)
            out += b"%d 0 obj\n" % n + self.objs[n] + b"\nendobj\n"
        size = max(nums) + 2
        tr = b"/Size %d /Root %d 0 R" % (size, root) + (b" /Info %d 0 R" % info if info else b"") + trailer_extra
        if xref_stream:
            xnum = size - 1
            rows = bytearray(b"\x00" + struct.pack(">IH", 0, 65535))
            for n in range(1, xnum):
                if n in in_stm:
                    rows += b"\x02" + struct.pack(">IH", stm_num, in_stm[n])
                elif n in offsets:
                    rows += b"\x01" + struct.pack(">IH", offsets[n], 0)
                else:
                    rows += b"\x00" + struct.pack(">IH", 0, 0)
            offsets[xnum] = len(out)
            rows += b"\x01" + struct.pack(">IH", len(out), 0)
            data = zlib.compress(bytes(rows))
            out += b"%d 0 obj\n<< /Type /XRef /W [1 4 2] /Filter /FlateDecode " % xnum + tr.replace(b"/Size %d" % size, b"/Size %d" % (xnum + 1)) + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream\nendobj\n"
            out += b"startxref\n%d\n%%%%EOF\n" % offsets[xnum]
            return bytes(out)
        xref = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (size - 1)
        for n in range(1, size - 1):
            out += (b"%010d 00000 n \n" % offsets[n]) if n in offsets else b"0000000000 65535 f \n"
        out += b"trailer\n<< " + tr + b" >>\nstartxref\n%d\n%%%%EOF\n" % xref
        return bytes(out)


def pdf_string(s):
    """a text string: PDFDocEncoding when Latin-1 does, else UTF-16BE"""
    try:
        b = s.encode("latin-1")
    except UnicodeEncodeError:
        b = b"\xfe\xff" + s.encode("utf-16-be")
    return b"(" + b.replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)") + b")"


def base_doc(text_pages, title="Lesetest", extra_catalog=b"", font=b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"):
    p = Pdf()
    p.add(b"", 1)
    p.add(b"", 2)
    fnum = p.add(font, 3)
    kids = []
    for t in text_pages:
        content = b"BT /F1 12 Tf 50 750 Td 14 TL\n"
        for line in t.split("\n"):
            content += b"(" + line.encode("cp1252").replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)") + b") '\n"
        content += b"ET\n"
        c = p.add(Pdf.stream(b"", content))
        kids.append(p.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents %d 0 R /Resources << /Font << /F1 %d 0 R >> >> >>" % (c, fnum)))
    p.objs[2] = b"<< /Type /Pages /Kids [" + b" ".join(b"%d 0 R" % k for k in kids) + b"] /Count %d >>" % len(kids)
    p.objs[1] = b"<< /Type /Catalog /Pages 2 0 R " + extra_catalog + b">>"
    info = p.add(b"<< /Title " + pdf_string(title) + b" /Author (JFLib) /CreationDate (D:20261123120000+01'00') >>")
    return p, info


PAGES_DE = ["Erste Seite: Grüße aus dem Lesetest.\nZweite Zeile mit Umlauten: Ärger, Öl, Übermaß.",
            "Seite zwei – mit Gedankenstrich und „Anführungszeichen“.\nDrei × vier = zwölf? Ja – €uro‰."]

# ------------------------------------------------------------------ encryption (for the fixtures)

PAD = bytes.fromhex("28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A")


def rc4(key, data):
    s = list(range(256))
    j = 0
    for i in range(256):
        j = (j + s[i] + key[i % len(key)]) & 0xFF
        s[i], s[j] = s[j], s[i]
    out = bytearray()
    i = j = 0
    for c in data:
        i = (i + 1) & 0xFF
        j = (j + s[i]) & 0xFF
        s[i], s[j] = s[j], s[i]
        out.append(c ^ s[(s[i] + s[j]) & 0xFF])
    return bytes(out)


def aes_cbc(key, iv, data, encrypt=True):
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
    c = Cipher(algorithms.AES(key), modes.CBC(iv))
    x = c.encryptor() if encrypt else c.decryptor()
    return x.update(data) + x.finalize()


def aes_ecb(key, data):
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
    x = Cipher(algorithms.AES(key), modes.ECB()).encryptor()
    return x.update(data) + x.finalize()


def hash6(pw, salt, udata, r):
    k = hashlib.sha256(pw + salt + udata).digest()
    if r == 5:
        return k
    i = 0
    while True:
        k1 = (pw + k + udata) * 64
        e = aes_cbc(k[:16], k[16:32], k1)
        n = sum(e[:16]) % 3
        k = [hashlib.sha256, hashlib.sha384, hashlib.sha512][n](e).digest()
        i += 1
        if i >= 64 and e[-1] <= i - 32:
            return k[:32]


def encrypt_doc(p, root, info, r, user, owner, aes, meta=True, seed=b"jflib"):
    """encrypt the strings and streams of Pdf p with the standard handler, revision r"""
    rnd = lambda n, tag: hashlib.sha512(seed + tag).digest()[:n]
    id0 = rnd(16, b"id")
    perms = -1340  # print, copy, no modify (bits 3, 5 set; 4 cleared ...)
    P = struct.pack("<i", perms)
    if r >= 5:
        key = rnd(32, b"key")
        uv, uk, ov, ok = rnd(8, b"uv"), rnd(8, b"uk"), rnd(8, b"ov"), rnd(8, b"ok")
        U = hash6(user, uv, b"", r) + uv + uk
        UE = aes_cbc(hash6(user, uk, b"", r), b"\0" * 16, key)
        O = hash6(owner, ov, U, r) + ov + ok
        OE = aes_cbc(hash6(owner, ok, U, r), b"\0" * 16, key)
        Perms = aes_ecb(key, P + b"\xff\xff\xff\xff" + (b"T" if meta else b"F") + b"adb" + rnd(4, b"pr"))
        enc = (b"<< /Filter /Standard /V 5 /R %d /Length 256 /CF << /StdCF << /CFM /AESV3 /AuthEvent /DocOpen /Length 32 >> >> /StmF /StdCF /StrF /StdCF " % r
               + b"/O <" + O.hex().encode() + b"> /U <" + U.hex().encode() + b"> /OE <" + OE.hex().encode() + b"> /UE <" + UE.hex().encode()
               + b"> /Perms <" + Perms.hex().encode() + b"> /P %d" % perms + (b"" if meta else b" /EncryptMetadata false") + b" >>")
        objkey = lambda num: key
    else:
        n = 5 if r == 2 else 16
        pad = lambda pw: (pw + PAD)[:32]
        h = hashlib.md5(pad(owner)).digest()
        if r >= 3:
            for _ in range(50):
                h = hashlib.md5(h[:n]).digest()
        O = rc4(h[:n], pad(user))
        if r >= 3:
            for i in range(1, 20):
                O = rc4(bytes(c ^ i for c in h[:n]), O)
        m = hashlib.md5(pad(user) + O + P + id0 + (b"\xff\xff\xff\xff" if r >= 4 and not meta else b""))
        key = m.digest()
        if r >= 3:
            for _ in range(50):
                key = hashlib.md5(key[:n]).digest()
        key = key[:n]
        if r == 2:
            U = rc4(key, PAD)
        else:
            x = hashlib.md5(PAD + id0).digest()
            x = rc4(key, x)
            for i in range(1, 20):
                x = rc4(bytes(c ^ i for c in key), x)
            U = x + rnd(16, b"upad")
        if r == 2:
            enc = b"<< /Filter /Standard /V 1 /R 2 /O <%s> /U <%s> /P %d >>" % (O.hex().encode(), U.hex().encode(), perms)
        elif r == 3:
            enc = b"<< /Filter /Standard /V 2 /R 3 /Length 128 /O <%s> /U <%s> /P %d >>" % (O.hex().encode(), U.hex().encode(), perms)
        else:
            cfm = b"/AESV2" if aes else b"/V2"
            enc = (b"<< /Filter /Standard /V 4 /R 4 /Length 128 /CF << /StdCF << /CFM " + cfm + b" /AuthEvent /DocOpen /Length 16 >> >> /StmF /StdCF /StrF /StdCF"
                   + b" /O <%s> /U <%s> /P %d" % (O.hex().encode(), U.hex().encode(), perms) + (b"" if meta else b" /EncryptMetadata false") + b" >>")

        def objkey(num):
            k = hashlib.md5(key + struct.pack("<I", num)[:3] + b"\0\0" + (b"sAlT" if aes else b"")).digest()
            return k[:min(n + 5, 16)]

    def crypt(num, data):
        k = objkey(num)
        if aes or r >= 5:
            iv = rnd(16, b"iv%d-%d" % (num, len(data)) + hashlib.md5(data).digest())
            padn = 16 - len(data) % 16
            return iv + aes_cbc(k, iv, data + bytes([padn]) * padn)
        return rc4(k, data)

    def esc_strings(num, body):
        # literal strings (...) without nesting, as base_doc and pdf_string write them
        def lit(m):
            raw = m.group(1)
            raw = re.sub(rb"\\([\\()])", rb"\1", raw)
            return b"<" + crypt(num, raw).hex().encode() + b">"
        return re.sub(rb"\(((?:[^()\\]|\\.)*)\)", lit, body)

    for num in list(p.objs):
        body = p.objs[num]
        if b"stream\n" in body:
            head, rest = body.split(b"stream\n", 1)
            data = rest[: rest.rindex(b"\nendstream")]
            if not meta and b"/Type /Metadata" in head:
                continue
            e = crypt(num, data)
            head = re.sub(rb"/Length \d+", b"/Length %d" % len(e), head)
            p.objs[num] = esc_strings(num, head) + b"stream\n" + e + b"\nendstream"
        else:
            p.objs[num] = esc_strings(num, body)
    enum = p.add(enc)
    return enum, id0


def fixtures():
    FIXTURES.mkdir(parents=True, exist_ok=True)
    cases = [
        ("r2_rc4_40.pdf", 2, False, True),
        ("r3_rc4_128.pdf", 3, False, True),
        ("r4_rc4_crypt_filter.pdf", 4, False, False),
        ("r4_aes_128.pdf", 4, True, True),
        ("r5_aes_256.pdf", 5, True, True),
        ("r6_aes_256.pdf", 6, True, False),
    ]
    made = []
    for name, r, aes, meta in cases:
        for user, suffix in ((b"user", ""), (b"", "_no_user_pw")):
            if suffix and r not in (3, 6):
                continue
            meta_stream = Pdf.stream(b"/Type /Metadata /Subtype /XML", b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><!-- Metadaten --></x:xmpmeta>", compress=False)
            p, info = base_doc(PAGES_DE, title="Verschlüsselt R%d" % r)
            mnum = p.add(meta_stream)
            outl = p.add(b"")
            item = p.add(b"<< /Title (Kapitel eins) /Parent %d 0 R /Dest [5 0 R /Fit] >>" % outl)
            p.objs[outl] = b"<< /Type /Outlines /First %d 0 R /Last %d 0 R /Count 1 >>" % (item, item)
            p.objs[1] = p.objs[1][:-2] + b"/Metadata %d 0 R /Outlines %d 0 R >>" % (mnum, outl)
            enum, id0 = encrypt_doc(p, 1, info, r, user, b"owner", aes, meta, seed=name.encode() + suffix.encode())
            data = p.write(1, info, trailer_extra=b" /Encrypt %d 0 R /ID [<%s> <%s>]" % (enum, id0.hex().encode(), id0.hex().encode()))
            f = FIXTURES / (name[:-4] + suffix + ".pdf")
            f.write_bytes(data)
            made.append((f, user.decode()))
    # Poppler must read each, with the user password and with the owner one
    bad = 0
    for f, user in made:
        for pw, opt in ((user, "-upw"), ("owner", "-opw")):
            out = subprocess.run(["pdftotext", opt, pw, str(f), "-"], capture_output=True)
            ok = out.returncode == 0 and "Grüße" in out.stdout.decode("utf-8", "replace")
            print(f"{f.name:34} {opt} {pw or '(empty)':6} {'ok' if ok else 'POPPLER FAILS: ' + out.stderr.decode()[:200]}")
            bad += not ok
    return 1 if bad else 0


# ------------------------------------------------------------------ the corpus

def made_here(d):
    """PDFs with the structures a reader must know, written above"""
    files = []

    def put(name, data):
        f = d / name
        f.write_bytes(data)
        files.append(f)

    p, info = base_doc(PAGES_DE)
    put("table.pdf", p.write(1, info))
    p, info = base_doc(PAGES_DE)
    put("xref_stream.pdf", p.write(1, info, version=b"1.5", xref_stream=True))
    p, info = base_doc(PAGES_DE)
    put("objstm.pdf", p.write(1, info, version=b"1.5", xref_stream=True, objstm=True))
    # an incremental update: the title changes, a page is added
    p, info = base_doc(PAGES_DE)
    data = p.write(1, info)
    prev = int(data[data.rindex(b"startxref") + 10:].split()[0])
    upd = bytearray(data)
    new_info = len(upd)
    upd += b"%d 0 obj\n<< /Title (Nach dem Update) /Author (JFLib) >>\nendobj\n" % info
    cnum = max(p.objs) + 1
    c_off = len(upd)
    content = zlib.compress(b"BT /F1 12 Tf 50 750 Td (Die dritte Seite kam mit dem Update.) Tj ET")
    upd += b"%d 0 obj\n<< /Filter /FlateDecode /Length %d >>\nstream\n" % (cnum, len(content)) + content + b"\nendstream\nendobj\n"
    pg_off = len(upd)
    upd += b"%d 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents %d 0 R /Resources << /Font << /F1 3 0 R >> >> >>\nendobj\n" % (cnum + 1, cnum)
    pages_off = len(upd)
    kids = re.search(rb"/Kids \[([^\]]*)\]", p.objs[2]).group(1)
    upd += b"2 0 obj\n<< /Type /Pages /Kids [" + kids + b" %d 0 R] /Count 3 >>\nendobj\n" % (cnum + 1)
    x = len(upd)
    upd += b"xref\n0 1\n0000000000 65535 f \n2 1\n%010d 00000 n \n%d 1\n%010d 00000 n \n%d 2\n%010d 00000 n \n%010d 00000 n \n" % (pages_off, info, new_info, cnum, c_off, pg_off)
    upd += b"trailer\n<< /Size %d /Root 1 0 R /Info %d 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (cnum + 2, info, prev, x)
    put("incremental.pdf", bytes(upd))
    # damaged: every offset of the table is 7 bytes off; then no table at all; then cut off
    good = p.write(1, info)
    put("shifted.pdf", good.replace(b"%PDF-1.7\n", b"%PDF-1.7\n%junk!\n", 1))
    put("no_xref.pdf", good[: good.index(b"xref\n")] + b"%%EOF\n")
    put("cut.pdf", good[: good.index(b"xref\n") - 40])
    # a composite font (Identity-H) with a ToUnicode CMap, glyphs as two-byte codes
    tu = (b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /JF def\n"
          b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n"
          b"2 beginbfrange <0001> <001A> <0041> <0020> <0022> [<0020> <00DF> <03A9>] endbfrange\n"
          b"1 beginbfchar <0030> <FB01> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end")
    q = Pdf()
    q.add(b"<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /r >> 1 << /S /D /P (A-) /St 7 >>] >> >>", 1)
    q.add(b"<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 >>", 2)
    tun = q.add(Pdf.stream(b"", tu), 6)
    q.add(b"<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Demo /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Demo "
          b"/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 600 /W [1 [700 650] 32 34 600] >> ] /ToUnicode %d 0 R >>" % tun, 3)
    hexs = lambda s: b"<" + s.encode() + b">"
    c1 = q.add(Pdf.stream(b"", b"BT /F 20 Tf 40 700 Td " + hexs("0008000500120012000F") + b" Tj 0 -30 Td [" + hexs("0010") + b" -2000 " + hexs("0021") + b"] TJ " + hexs("0030") + b" Tj ET"))
    c2 = q.add(Pdf.stream(b"", b"BT /F 20 Tf 40 700 Td " + hexs("0013000F000E0005") + b" Tj ET"))
    q.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 800] /Contents %d 0 R /Resources << /Font << /F 3 0 R >> >> >>" % c1, 4)
    q.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 800] /Rotate 90 /Contents %d 0 R /Resources << /Font << /F 3 0 R >> >> >>" % c2, 5)
    put("type0.pdf", q.write(1))
    return files


def ghostscript(d, files):
    out = []
    ps = d / "fonts.ps"
    lines = ["%!PS", "/F { findfont exch scalefont setfont } def"]
    y = 750
    for font in ["Helvetica", "Helvetica-Bold", "Times-Roman", "Times-Italic", "Courier", "Palatino-Roman", "Bookman-Demi"]:
        lines.append(f"14 /{font} F 50 {y} moveto (Text in {font}: The quick brown fox jumps over the lazy dog.) show")
        y -= 24
    lines += ["showpage", "12 /Times-Roman F 50 750 moveto (Second page: page two of the PostScript file.) show showpage"]
    ps.write_text("\n".join(lines) + "\n")
    for lvl, extra, name in (("1.4", [], "gs14.pdf"), ("1.7", ["-dWriteObjStms=true", "-dWriteXRefStm=true"], "gs17.pdf")):
        f = d / name
        r = subprocess.run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite", f"-dCompatibilityLevel={lvl}", *extra, f"-sOutputFile={f}", str(ps)], capture_output=True)
        if r.returncode == 0:
            out.append(f)
    for src in files:
        f = d / ("gs_" + src.name)
        r = subprocess.run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite", "-dCompatibilityLevel=1.7", f"-sOutputFile={f}", str(src)], capture_output=True, timeout=120)
        if r.returncode == 0 and f.exists() and f.stat().st_size > 0:
            out.append(f)
    out = [(f, None, None) for f in out]
    # Ghostscript's own encryption (RC4, 128 bit)
    f = d / "gs_encrypted.pdf"
    r = subprocess.run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite", "-sOwnerPassword=owner", "-sUserPassword=user", "-dEncryptionR=3", "-dKeyLength=128",
                        "-dPermissions=-1044", f"-sOutputFile={f}", str(ps)], capture_output=True)
    if r.returncode == 0:
        out += [(f, "user", "-upw"), (f, "owner", "-opw")]
    # man pages through groff's PostScript
    if shutil.which("groff"):
        for page in ["gzip", "tar", "grep", "bash"]:
            src = Path(f"/usr/share/man/man1/{page}.1.gz")
            if not src.exists():
                continue
            ps_page = d / f"man_{page}.ps"
            with open(ps_page, "wb") as w:
                z = subprocess.run(["zcat", str(src)], capture_output=True).stdout
                w.write(subprocess.run(["groff", "-Tps", "-man"], input=z, capture_output=True).stdout)
            f = d / f"man_{page}.pdf"
            r = subprocess.run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite", f"-sOutputFile={f}", str(ps_page)], capture_output=True, timeout=300)
            if r.returncode == 0 and f.exists():
                out.append((f, None, None))
    return out


def cairo(d, files):
    """the files written again by Poppler's cairo backend (other fonts, Type 3 for some)"""
    out = []
    for src in files:
        f = d / ("cairo_" + src.name)
        r = subprocess.run(["pdftocairo", "-pdf", str(src), str(f)], capture_output=True, timeout=300)
        if r.returncode == 0 and f.exists():
            out.append((f, None, None))
    return out


def libreoffice(d):
    html = d / "page.html"
    html.write_text("""<!DOCTYPE html><html><head><meta charset="utf-8"><title>LibreOffice-Seite</title></head><body>
<h1>Überschrift eins</h1><p>Ein Absatz mit Umlauten: Grüße, Öl, Ärger, Übermaß – und ein <a href="https://example.org/">Link</a>.</p>
<h2>Zweite Ebene</h2><p>Griechisch: Καλημέρα κόσμε. Kyrillisch: Здравствуй, мир. Hebräisch: שלום עולם.</p>
<table border="1"><tr><td>Zelle A1</td><td>Zelle B1</td></tr><tr><td>Zelle A2</td><td>Zelle B2</td></tr></table>
<h2>Formular</h2><form><p>Name: <input type="text" name="vorname" value="Erika"></p>
<p><input type="checkbox" name="zustimmung" checked> Zustimmung</p>
<p><select name="farbe"><option>Rot</option><option selected>Grün</option></select></p></form>
<p>Ligaturen: office, fliegen, Pfiff. Zahlen: 3 × 4 ≥ 10 €.</p></body></html>""", encoding="utf-8")
    enc = d / "page_encrypted.html"
    shutil.copy(html, enc)
    csv = d / "sheet.csv"
    csv.write_text("Artikel,Menge,Preis\nÄpfel,3,1.20\nBirnen,5,0.95\nKirschen,12,4.50\n", encoding="utf-8")
    env = dict(os.environ, HOME=str(d))
    secret = 'pdf:writer_web_pdf_Export:{"EncryptFile":{"type":"boolean","value":"true"},"DocumentOpenPassword":{"type":"string","value":"user"},"RestrictPermissions":{"type":"boolean","value":"true"},"PermissionPassword":{"type":"string","value":"owner"}}'
    for src, conv in ((html, "pdf:writer_web_pdf_Export"), (enc, secret), (csv, "pdf:calc_pdf_Export")):
        subprocess.run(["soffice", "--headless", "--norestore", "--convert-to", conv, "--outdir", str(d), str(src)], capture_output=True, timeout=180, env=env)
    out = []
    if (d / "page.pdf").exists():
        out.append((d / "page.pdf", None, None))
    if (d / "page_encrypted.pdf").exists():
        out += [(d / "page_encrypted.pdf", "user", "-upw"), (d / "page_encrypted.pdf", "owner", "-opw")]
    if (d / "sheet.pdf").exists():
        out.append((d / "sheet.pdf", None, None))
    return out


def system_pdfs():
    out = []
    for base in ["/usr/share/doc", "/usr/share/cups", "/usr/share/texmf", "/usr/share/texlive"]:
        for root, _, names in os.walk(base):
            if root.count(os.sep) > 8:
                continue
            for n in names:
                if n.lower().endswith(".pdf"):
                    out.append(Path(root, n))
    return sorted(out)[:60]


# ------------------------------------------------------------------ comparison

# where Poppler's text is the wrong one, for files of the corpus: (the least word agreement, why)
KNOWN = {
    "sheet.pdf": (0.9, 'Poppler joins two numbers of neighbouring cells 2 pt apart ("12", "4.50" -> "124.50")'),
}

def words(t):
    """the words of a text; three or more single letters in a row are one word (Poppler writes
    text turned by 45 degrees letter by letter)"""
    t = unicodedata.normalize("NFKC", t)
    out, run = [], []
    for w in re.findall(r"\w+", t.lower()):
        if len(w) == 1 and w.isalpha():
            run.append(w)
            continue
        out.extend([''.join(run)] if len(run) >= 3 else run)
        run = []
        out.append(w)
    out.extend([''.join(run)] if len(run) >= 3 else run)
    return collections.Counter(out)


def similarity(a, b):
    wa, wb = words(a), words(b)
    total = sum(wa.values()) + sum(wb.values())
    if total == 0:
        return 1.0
    return 2 * sum((wa & wb).values()) / total


def poppler(f, pw, opt):
    args = [opt, pw] if pw is not None else []
    info = subprocess.run(["pdfinfo", *args, "-f", "1", "-l", "9999", str(f)], capture_output=True, timeout=120)
    if info.returncode != 0:
        return None
    text = info.stdout.decode("utf-8", "replace")
    pages = int(re.search(r"^Pages:\s+(\d+)", text, re.M).group(1))
    sizes = [(float(m.group(1)), float(m.group(2)), int(m.group(3) or 0)) for m in re.finditer(r"^Page\s+\d+ size:\s+([\d.]+) x ([\d.]+).*\n(?:Page\s+\d+ rot:\s+(\d+))?", text, re.M)]
    field = lambda k: (re.search(rf"^{k}:[ \t]*(.*)$", text, re.M) or [None, None])[1]
    out = subprocess.run(["pdftotext", *args, "-raw", "-enc", "UTF-8", str(f), "-"], capture_output=True, timeout=300)
    ptext = out.stdout.decode("utf-8", "replace").split("\f")
    det = subprocess.run(["pdfdetach", *args, "-list", str(f)], capture_output=True, timeout=60)
    natt = 0
    m = re.match(r"(\d+) embedded files", det.stdout.decode("utf-8", "replace"))
    if m:
        natt = int(m.group(1))
    return {"pages": pages, "sizes": sizes, "title": field("Title"), "author": field("Author"), "encrypted": (field("Encrypted") or "no").startswith("yes"),
            "text": ptext, "attachments": natt}


def check(exe, f, pw, opt, min_sim, verbose, ref=None):
    """`ref`: the intact file a damaged one is compared with (when Poppler cannot read it)"""
    args = [str(exe), "--read", str(f)] + (["--password", pw] if pw else [])
    try:
        r = subprocess.run(args, capture_output=True, timeout=300)
    except subprocess.TimeoutExpired:
        return ["timeout"]
    if r.returncode not in (0, 1):
        return [f"pdftest crashed (exit {r.returncode}): {r.stderr.decode()[-300:]}"]
    try:
        ours = json.loads(r.stdout.decode("utf-8"))
    except ValueError:
        return [f"no JSON: {r.stdout[:200]!r} {r.stderr[-200:]!r}"]
    pop = poppler(f, pw, opt)
    if pop is None and ref is not None:
        pop = poppler(ref, None, None)
        if pop is not None:
            pop["title"] = pop["author"] = None  # the damage may have taken the information
    if pop is None:
        return []
    if "error" in ours:
        return [f"pdf::read: {ours['error']}"]
    bad = []
    if ours["pages"] != pop["pages"]:
        bad.append(f"pages {ours['pages']} vs Poppler {pop['pages']}")
    for i, (a, b) in enumerate(zip(ours["sizes"], pop["sizes"])):
        w, h = (b[0], b[1]) if b[2] % 180 == 0 else (b[1], b[0])
        if abs(a[0] - w) > 1 or abs(a[1] - h) > 1:
            bad.append(f"page {i + 1} size {a[0]}x{a[1]} vs Poppler {w}x{h} (rot {b[2]})")
            break
    for k in ("title", "author"):
        if pop[k] is not None and (ours["info"][k] or "").strip() != pop[k].strip():
            bad.append(f"{k} {ours['info'][k]!r} vs Poppler {pop[k]!r}")
    if ours["encrypted"] != pop["encrypted"]:
        bad.append(f"encrypted {ours['encrypted']} vs Poppler {pop['encrypted']}")
    if len(ours["attachments"]) != pop["attachments"]:
        bad.append(f"attachments {len(ours['attachments'])} vs Poppler {pop['attachments']}")
    sims = []
    for i, t in enumerate(ours["text"]):
        p = pop["text"][i] if i < len(pop["text"]) else ""
        s = similarity(t, p)
        sims.append(s)
        if s < min_sim:
            bad.append(f"page {i + 1}: words agree {s:.3f}")
            if verbose:
                wa, wb = words(t), words(p)
                bad.append(f"    only ours: {list((wa - wb).elements())[:25]}")
                bad.append(f"    only Poppler: {list((wb - wa).elements())[:25]}")
    return bad, (min(sims) if sims else 1.0), ours


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "build/linux/pdftest"))
    ap.add_argument("--keep")
    ap.add_argument("--min", type=float, default=0.97)
    ap.add_argument("--fixtures", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    a = ap.parse_args()
    if a.fixtures:
        return fixtures()
    for tool in ("pdfinfo", "pdftotext", "pdfdetach"):
        if not shutil.which(tool):
            print(f"pdf_read_check: {tool} (Poppler) is not installed")
            return 1
    d = Path(a.keep) if a.keep else Path(tempfile.mkdtemp(prefix="pdf_read_check_"))
    d.mkdir(parents=True, exist_ok=True)
    corpus = []  # (file, password, poppler option)
    sample = d / "sample.pdf"
    subprocess.run([a.exe, "--sample", str(sample)], capture_output=True)
    made = [sample] + made_here(d)
    for f in made:
        corpus.append((f, None, None))
    for f in sorted(FIXTURES.glob("*.pdf")):
        user = "" if "no_user_pw" in f.name else "user"
        corpus.append((f, user or None, "-upw"))
        corpus.append((f, "owner", "-opw"))
    if shutil.which("gs"):
        corpus += ghostscript(d, [m for m in made if m.name not in ("cut.pdf",)])
    else:
        print("pdf_read_check: Ghostscript is not installed, skipped")
    if shutil.which("soffice"):
        corpus += libreoffice(d)
    else:
        print("pdf_read_check: LibreOffice is not installed, skipped")
    if shutil.which("pdfunite"):
        u = d / "united.pdf"
        if subprocess.run(["pdfunite", str(sample), str(d / "table.pdf"), str(d / "type0.pdf"), str(u)], capture_output=True).returncode == 0:
            corpus.append((u, None, None))
    if shutil.which("pdftocairo"):
        corpus += cairo(d, [p for p in (sample, d / "table.pdf", d / "type0.pdf", d / "gs14.pdf", d / "page.pdf") if p.exists()])
    corpus += [(f, None, None) for f in system_pdfs()]
    # damaged files Poppler gives up on: compared with the file before the damage
    refs = {"no_xref.pdf": d / "table.pdf", "cut.pdf": d / "table.pdf"}
    failed = 0
    for f, pw, opt in corpus:
        res = check(a.exe, f, pw, opt, a.min, a.verbose, refs.get(f.name) if f.parent == d else None)
        name = f.name if f.parent in (d, FIXTURES) else str(f)
        tag = f" [{opt} {pw or '(empty)'}]" if opt else ""
        if isinstance(res, list):
            if res:
                failed += 1
                print(f"FAIL {name}{tag}: {'; '.join(res)}")
            else:
                print(f"skip {name}{tag}: Poppler cannot read it either")
            continue
        bad, sim, ours = res
        extra = f"{ours['pages']:3} pages, words {sim:.3f}" + (", repaired" if ours.get("repaired") else "") + (f", R{ours['revision']}" if ours.get("revision") else "")
        known = KNOWN.get(f.name) if f.parent == d else None
        if bad and known and sim >= known[0]:
            print(f"ok   {name}{tag}: {extra} (Poppler differs: {known[1]})")
        elif bad:
            failed += 1
            print(f"FAIL {name}{tag}: {extra}")
            for b in bad:
                print(f"     {b}")
        else:
            print(f"ok   {name}{tag}: {extra}")
    print(f"pdf_read_check: {len(corpus) - failed} of {len(corpus)} files agree with Poppler" + ("" if a.keep else f" (corpus in {d}, removed)"))
    if not a.keep:
        shutil.rmtree(d, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
