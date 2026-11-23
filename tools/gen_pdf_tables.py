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
  - for drawing glyphs: the glyph names of those encodings and of the ZapfDingbats font's
    built-in encoding (gs_dbt_e.ps), and the 391 standard strings of CFF (Adobe Technical Note
    5176, Appendix A; written out below and checked against the encodings)

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

    for f in ["gs_std_e.ps", "gs_il1_e.ps", "gs_wan_e.ps", "gs_mro_e.ps", "gs_mex_e.ps", "gs_pdf_e.ps", "gs_sym_e.ps", "gs_dbt_e.ps"]:
        parse(f)
    return encs


# CFF's standard strings (SID 0 to 390): the ISOAdobe charset (0 to 228), the expert names, the
# names of the weights
CFF_STRINGS = (""".notdef space exclam quotedbl numbersign dollar percent ampersand quoteright parenleft
parenright asterisk plus comma hyphen period slash zero one two three four five six seven eight nine
colon semicolon less equal greater question at A B C D E F G H I J K L M N O P Q R S T U V W X Y Z
bracketleft backslash bracketright asciicircum underscore quoteleft a b c d e f g h i j k l m n o p q
r s t u v w x y z braceleft bar braceright asciitilde exclamdown cent sterling fraction yen florin
section currency quotesingle quotedblleft guillemotleft guilsinglleft guilsinglright fi fl endash
dagger daggerdbl periodcentered paragraph bullet quotesinglbase quotedblbase quotedblright
guillemotright ellipsis perthousand questiondown grave acute circumflex tilde macron breve dotaccent
dieresis ring cedilla hungarumlaut ogonek caron emdash AE ordfeminine Lslash Oslash OE ordmasculine
ae dotlessi lslash oslash oe germandbls onesuperior logicalnot mu trademark Eth onehalf plusminus
Thorn onequarter divide brokenbar degree thorn threequarters twosuperior registered minus eth
multiply threesuperior copyright Aacute Acircumflex Adieresis Agrave Aring Atilde Ccedilla Eacute
Ecircumflex Edieresis Egrave Iacute Icircumflex Idieresis Igrave Ntilde Oacute Ocircumflex Odieresis
Ograve Otilde Scaron Uacute Ucircumflex Udieresis Ugrave Yacute Ydieresis Zcaron aacute acircumflex
adieresis agrave aring atilde ccedilla eacute ecircumflex edieresis egrave iacute icircumflex
idieresis igrave ntilde oacute ocircumflex odieresis ograve otilde scaron uacute ucircumflex
udieresis ugrave yacute ydieresis zcaron exclamsmall Hungarumlautsmall dollaroldstyle dollarsuperior
ampersandsmall Acutesmall parenleftsuperior parenrightsuperior twodotenleader onedotenleader
zerooldstyle oneoldstyle twooldstyle threeoldstyle fouroldstyle fiveoldstyle sixoldstyle
sevenoldstyle eightoldstyle nineoldstyle commasuperior threequartersemdash periodsuperior
questionsmall asuperior bsuperior centsuperior dsuperior esuperior isuperior lsuperior msuperior
nsuperior osuperior rsuperior ssuperior tsuperior ff ffi ffl parenleftinferior parenrightinferior
Circumflexsmall hyphensuperior Gravesmall Asmall Bsmall Csmall Dsmall Esmall Fsmall Gsmall Hsmall
Ismall Jsmall Ksmall Lsmall Msmall Nsmall Osmall Psmall Qsmall Rsmall Ssmall Tsmall Usmall Vsmall
Wsmall Xsmall Ysmall Zsmall colonmonetary onefitted rupiah Tildesmall exclamdownsmall centoldstyle
Lslashsmall Scaronsmall Zcaronsmall Dieresissmall Brevesmall Caronsmall Dotaccentsmall Macronsmall
figuredash hypheninferior Ogoneksmall Ringsmall Cedillasmall questiondownsmall oneeighth
threeeighths fiveeighths seveneighths onethird twothirds zerosuperior foursuperior fivesuperior
sixsuperior sevensuperior eightsuperior ninesuperior zeroinferior oneinferior twoinferior
threeinferior fourinferior fiveinferior sixinferior seveninferior eightinferior nineinferior
centinferior dollarinferior periodinferior commainferior Agravesmall Aacutesmall Acircumflexsmall
Atildesmall Adieresissmall Aringsmall AEsmall Ccedillasmall Egravesmall Eacutesmall Ecircumflexsmall
Edieresissmall Igravesmall Iacutesmall Icircumflexsmall Idieresissmall Ethsmall Ntildesmall
Ogravesmall Oacutesmall Ocircumflexsmall Otildesmall Odieresissmall OEsmall Oslashsmall Ugravesmall
Uacutesmall Ucircumflexsmall Udieresissmall Yacutesmall Thornsmall Ydieresissmall 001.000 001.001
001.002 001.003 Black Bold Book Light Medium Regular Roman Semibold""").split()


def check_cff_strings(encs):
    """the standard strings against what they must agree with: StandardEncoding's names in code
    order are SIDs 1 to 149, the expert names are MacExpertEncoding's"""
    assert len(CFF_STRINGS) == 391, len(CFF_STRINGS)
    assert len(set(CFF_STRINGS)) == 391
    std = [n for n in encs["StandardEncoding"][32:] if n != ".notdef"]
    assert CFF_STRINGS[1:150] == std, "the ISOAdobe names do not follow StandardEncoding"
    lat = set(encs["WinAnsiEncoding"]) | set(encs["MacRomanEncoding"]) | set(encs["ISOLatin1Encoding"])
    assert all(n in lat for n in CFF_STRINGS[150:229]), [n for n in CFF_STRINGS[150:229] if n not in lat]
    mex = set(encs["MacExpertEncoding"])
    extra = [n for n in CFF_STRINGS[229:379] if n not in mex]
    assert not extra, extra


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
    # glyph names: the encodings' and CFF's standard strings in one sorted pool
    check_cff_strings(encs)
    name_encs = [("STANDARD", "StandardEncoding"), ("WINANSI", "WinAnsiEncoding"), ("MACROMAN", "MacRomanEncoding"),
                 ("MACEXPERT", "MacExpertEncoding"), ("SYMBOL", "SymbolEncoding"), ("DINGBATS", "DingbatsEncoding")]
    pool = sorted(set(CFF_STRINGS).union(*(encs[e] for _, e in name_encs)))
    at = {n: i for i, n in enumerate(pool)}
    nblob, nstart = bytearray(), []
    for n in pool:
        nstart.append(len(nblob))
        nblob += n.encode()
    nstart.append(len(nblob))
    o.append(f"/// the glyph names of the encodings and of CFF's standard strings, sorted, one after the other\npub(crate) static GLYPH_NAMES: &[u8] = b\"{nblob.decode()}\";\n")
    o.append(f"/// where each name starts in GLYPH_NAMES (and where the last ends)\npub(crate) static GLYPH_START: [u16; {len(nstart)}] = [\n")
    for i in range(0, len(nstart), 16):
        o.append("    " + ", ".join(str(v) for v in nstart[i:i + 16]) + ",\n")
    o.append("];\n")
    for rname, e in name_encs:
        vals = [0xFFFF if n == ".notdef" else at[n] for n in encs[e]]
        o.append(rust_u16(f"NAMES_{rname}", vals, f"{e}: each code's glyph name in GLYPH_NAMES (0xFFFF: none)"))
    o.append(rust_u16("CFF_STRINGS", [at[n] for n in CFF_STRINGS], "CFF's standard strings: each SID's name in GLYPH_NAMES"))
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
