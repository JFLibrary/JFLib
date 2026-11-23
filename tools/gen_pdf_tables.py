#!/usr/bin/env python3
"""gen_pdf_tables.py -- the tables of lib/pdf's reader: lib/pdf/src/read/tables.rs

  - the Adobe Glyph List 2.0 (glyph names -> Unicode; BSD-3-Clause, Adobe; the notice is in
    lib/pdf/licenses/AGL-LICENSE.txt), read from Ghostscript's copy (gs_agl.ps, which holds
    the list unchanged, its entries of several code points as comments)
  - the encodings of ISO 32000-1 Annex D as Unicode: StandardEncoding, WinAnsiEncoding,
    MacRomanEncoding, MacExpertEncoding, PDFDocEncoding and the built-in encoding of the Symbol
    font, from Ghostscript's encoding vectors (gs_*_e.ps) through the glyph list; PDFDocEncoding's
    undefined codes keep their Latin-1 value
  - the advance widths of Helvetica and Times (regular, bold, italic, bold italic) for the
    characters of those encodings, from the Liberation fonts (SIL OFL 1.1), whose widths match
    them: what a page in one of the 14 standard fonts without /Widths needs (Courier is 600)

  python tools/gen_pdf_tables.py [--check]     --check: fail when tables.rs is not what it writes

Nothing is downloaded; the sources are the files of Ghostscript and fonts-liberation.
"""
import argparse
import glob
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "lib/pdf/src/read/tables.rs"


def gs_dir():
    for d in sorted(glob.glob("/usr/share/ghostscript/*/Resource/Init")) + ["/usr/share/ghostscript/Resource/Init"]:
        if Path(d, "gs_agl.ps").exists():
            return Path(d)
    sys.exit("gen_pdf_tables: Ghostscript's Resource/Init (gs_agl.ps) is not installed")


def liberation(name):
    for d in ["/usr/share/fonts/truetype/liberation", "/usr/share/fonts/truetype/liberation2", "/usr/share/fonts/liberation"]:
        p = Path(d, name)
        if p.exists():
            return p
    sys.exit(f"gen_pdf_tables: {name} (fonts-liberation) is not installed")


def read_agl(path):
    agl = {}
    for line in path.read_text(encoding="latin-1").splitlines():
        if line.startswith(".dicttomark"):
            break
        m = re.match(r"^(?:% )?/(\S+)((?: 16#[0-9A-Fa-f]+)+)\s*$", line)
        if m:
            cps = [int(v[3:], 16) for v in m.group(2).split()]
            agl.setdefault(m.group(1), cps)
    return agl


def read_encodings(d):
    encs = {}

    def parse(fname):
        toks = re.sub(r"%[^\n]*", "", (d / fname).read_text(encoding="latin-1")).split()
        i = next(k for k, t in enumerate(toks) if re.match(r"^/\w+Encoding$", t))
        name = toks[i][1:]
        out, k = [], i + 1
        while k < len(toks):
            t = toks[k]
            if t == "256" and k + 1 < len(toks) and toks[k + 1] in ("packedarray", "vmstatus"):
                break
            if t.startswith("/"):
                out.append(t[1:])
                k += 1
            elif t.endswith("Encoding") and toks[k + 3] == "getinterval":
                s, n = int(toks[k + 1]), int(toks[k + 2])
                out.extend(encs[t][s:s + n])
                assert toks[k + 4] == "aload" and toks[k + 5] == "pop", fname
                k += 6
            else:
                k += 1
        assert len(out) == 256, (fname, len(out))
        encs[name] = out

    for f in ["gs_std_e.ps", "gs_il1_e.ps", "gs_wan_e.ps", "gs_mro_e.ps", "gs_mex_e.ps", "gs_pdf_e.ps", "gs_sym_e.ps"]:
        parse(f)
    return encs


def ttf_widths(path, chars):
    """unicode -> advance in 1/1000 em, from a TrueType font's cmap and hmtx"""
    b = path.read_bytes()
    n = struct.unpack(">H", b[4:6])[0]
    tab = {}
    for i in range(n):
        tag, _, off, ln = struct.unpack(">4sIII", b[12 + 16 * i:28 + 16 * i])
        tab[tag.decode()] = (off, ln)
    upem = struct.unpack(">H", b[tab["head"][0] + 18:tab["head"][0] + 20])[0]
    nhm = struct.unpack(">H", b[tab["hhea"][0] + 34:tab["hhea"][0] + 36])[0]
    hm = tab["hmtx"][0]
    adv = lambda g: struct.unpack(">H", b[hm + 4 * min(g, nhm - 1):hm + 4 * min(g, nhm - 1) + 2])[0]
    co = tab["cmap"][0]
    cmap = {}
    for i in range(struct.unpack(">H", b[co + 2:co + 4])[0]):
        pid, eid, off = struct.unpack(">HHI", b[co + 4 + 8 * i:co + 12 + 8 * i])
        if (pid, eid) not in ((3, 1), (0, 3)):
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
        for k in range(seg):
            for c in range(starts[k], ends[k] + 1):
                if c == 0xFFFF:
                    continue
                if ros[k] == 0:
                    g = (c + deltas[k]) & 0xFFFF
                else:
                    p = ro_at + 2 * k + ros[k] + 2 * (c - starts[k])
                    g = struct.unpack(">H", b[p:p + 2])[0]
                    if g:
                        g = (g + deltas[k]) & 0xFFFF
                if g:
                    cmap.setdefault(c, g)
        break
    return {c: round(adv(cmap[c]) * 1000 / upem) for c in sorted(chars) if c in cmap}


def rust_u16(name, vals, doc):
    rows = [", ".join(f"0x{v:04X}" for v in vals[i:i + 16]) for i in range(0, len(vals), 16)]
    return f"/// {doc}\npub(crate) static {name}: [u16; {len(vals)}] = [\n" + "".join(f"    {r},\n" for r in rows) + "];\n"


def generate():
    d = gs_dir()
    agl = read_agl(d / "gs_agl.ps")
    encs = read_encodings(d)

    def uni(names, latin1=False):
        out = []
        for code, n in enumerate(names):
            cps = agl.get(n)
            v = cps[0] if cps and len(cps) == 1 and cps[0] < 0x10000 else 0
            out.append(v if v or not latin1 else code)
        return out

    tables = {
        "STANDARD": (uni(encs["StandardEncoding"]), "StandardEncoding (Annex D.2)"),
        "WINANSI": (uni(encs["WinAnsiEncoding"]), "WinAnsiEncoding (Annex D.2)"),
        "MACROMAN": (uni(encs["MacRomanEncoding"]), "MacRomanEncoding (Annex D.2)"),
        "MACEXPERT": (uni(encs["MacExpertEncoding"]), "MacExpertEncoding (Annex D.4; small capitals and old-style figures in the private use area as the glyph list maps them)"),
        "PDFDOC": (uni(encs["PDFDocEncoding"], latin1=True), "PDFDocEncoding (Annex D.2; undefined codes as Latin-1)"),
        "SYMBOL": (uni(encs["SymbolEncoding"]), "the built-in encoding of the Symbol font (Annex D.5)"),
    }
    # PDFDocEncoding: Ghostscript leaves the undefined codes .notdef; nothing else to fix

    names = sorted(agl)
    blob = bytearray()
    starts, codes, seq = [], [], []
    for n in names:
        starts.append(len(blob))
        blob += n.encode()
        cps = agl[n]
        if len(cps) == 1:
            codes.append(cps[0])
        else:
            codes.append(0x8000_0000 | len(seq) << 8 | len(cps))
            seq.extend(cps)
    starts.append(len(blob))

    latin = set()
    for t in ("STANDARD", "WINANSI", "MACROMAN", "PDFDOC"):
        latin.update(v for v in tables[t][0] if v)
    fonts = [
        ("HELVETICA", "LiberationSans-Regular.ttf"), ("HELVETICA_BOLD", "LiberationSans-Bold.ttf"),
        ("HELVETICA_OBLIQUE", "LiberationSans-Italic.ttf"), ("HELVETICA_BOLD_OBLIQUE", "LiberationSans-BoldItalic.ttf"),
        ("TIMES", "LiberationSerif-Regular.ttf"), ("TIMES_BOLD", "LiberationSerif-Bold.ttf"),
        ("TIMES_ITALIC", "LiberationSerif-Italic.ttf"), ("TIMES_BOLD_ITALIC", "LiberationSerif-BoldItalic.ttf"),
    ]

    o = ["// generated by tools/gen_pdf_tables.py -- do not edit\n",
         "// The Adobe Glyph List 2.0 (BSD-3-Clause, Adobe; lib/pdf/licenses/AGL-LICENSE.txt), the encodings\n",
         "// of ISO 32000-1 Annex D as Unicode, the widths of Helvetica and Times as the Liberation fonts\n",
         "// (SIL OFL 1.1) have them.\n\n"]
    o.append(f"/// the glyph names of the list, sorted, one after the other\npub(crate) static AGL_NAMES: &[u8] = b\"{blob.decode()}\";\n")
    o.append(f"/// where each name starts in AGL_NAMES (and where the last ends)\npub(crate) static AGL_START: [u32; {len(starts)}] = [\n")
    for i in range(0, len(starts), 16):
        o.append("    " + ", ".join(str(v) for v in starts[i:i + 16]) + ",\n")
    o.append("];\n")
    o.append(f"/// each name's code point, or 0x8000_0000 | index in AGL_SEQ << 8 | count\npub(crate) static AGL_CODE: [u32; {len(codes)}] = [\n")
    for i in range(0, len(codes), 12):
        o.append("    " + ", ".join(f"0x{v:X}" for v in codes[i:i + 12]) + ",\n")
    o.append("];\n")
    o.append(f"/// the code points of the names that stand for several\npub(crate) static AGL_SEQ: [u32; {len(seq)}] = [\n")
    for i in range(0, len(seq), 12):
        o.append("    " + ", ".join(f"0x{v:X}" for v in seq[i:i + 12]) + ",\n")
    o.append("];\n\n")
    for name, (vals, doc) in tables.items():
        o.append(rust_u16(name, vals, doc))
    o.append("\n")
    for name, file in fonts:
        w = ttf_widths(liberation(file), latin)
        o.append(f"/// {file[:-4]}: (char, width in 1/1000 em), sorted\npub(crate) static WIDTHS_{name}: [(u16, u16); {len(w)}] = [\n")
        items = list(w.items())
        for i in range(0, len(items), 10):
            o.append("    " + ", ".join(f"(0x{c:04X}, {v})" for c, v in items[i:i + 10]) + ",\n")
        o.append("];\n")
    return "".join(o)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    text = generate()
    if a.check:
        if not OUT.exists() or OUT.read_text() != text:
            print(f"gen_pdf_tables: {OUT.relative_to(ROOT)} is not what the sources give")
            return 1
        print("gen_pdf_tables: up to date")
        return 0
    OUT.write_text(text)
    print(f"gen_pdf_tables: {OUT.relative_to(ROOT)} ({len(text)} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
