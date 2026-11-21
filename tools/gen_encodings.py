#!/usr/bin/env python3
"""gen_encodings.py -- lib/formats/src/charsets.rs: the single-byte character sets of formats::encoding

    python tools/gen_encodings.py            write lib/formats/src/charsets.rs
    python tools/gen_encodings.py --check    exit 1 when the file is not what this script writes
    python tools/gen_encodings.py --verify   compare the tables and labels with the WHATWG data of
                                             encoding_rs (in Cargo's registry, when it is there) and
                                             with Node's TextDecoder (when node is on PATH)

The character sets are the single-byte encodings of the WHATWG Encoding Standard
(https://encoding.spec.whatwg.org/, the ones browsers, HTML and mail use) and the DOS code pages
of old files (DXF's $DWGCODEPAGE, ZIP names, DBF): IBM437, 737, 775, 850 ... 869, and an exact
ISO-8859-1.  Each table maps the bytes 0x80..0xFF to Unicode (0: unassigned), taken from
Python's codecs.  For the WHATWG ones two rules make them the standard's index: where Python
leaves a byte of 0x80..0x9F unassigned, WHATWG maps it to the C1 control of the same number (the
windows-* pages, as Windows itself does), and three entries differ (WHATWG_FIXES).  The labels
are the standard's (encodings.json), all of them, multi-byte ones included: formats::encoding
names what it cannot decode instead of not knowing it.  --verify checks all of this against the
standard's own data.

The labels and the WHATWG_FIXES come from the Encoding Standard: Copyright © WHATWG (Apple,
Google, Mozilla, Microsoft), in source code under the BSD 3-Clause License
(licenses/WHATWG-LICENSE.txt).
"""
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "lib" / "formats" / "src" / "charsets.rs"

# (variant, name, Windows code page, Python codec, WHATWG, description)
CHARSETS = [
    ("Ibm866", "IBM866", 866, "cp866", True, "DOS Cyrillic"),
    ("Iso8859_2", "ISO-8859-2", 28592, "iso8859_2", True, "Latin 2, Central European"),
    ("Iso8859_3", "ISO-8859-3", 28593, "iso8859_3", True, "Latin 3, South European"),
    ("Iso8859_4", "ISO-8859-4", 28594, "iso8859_4", True, "Latin 4, North European"),
    ("Iso8859_5", "ISO-8859-5", 28595, "iso8859_5", True, "Cyrillic"),
    ("Iso8859_6", "ISO-8859-6", 28596, "iso8859_6", True, "Arabic"),
    ("Iso8859_7", "ISO-8859-7", 28597, "iso8859_7", True, "Greek"),
    ("Iso8859_8", "ISO-8859-8", 28598, "iso8859_8", True, "Hebrew, visual order"),
    ("Iso8859_8I", "ISO-8859-8-I", 38598, "iso8859_8", True, "Hebrew, logical order (the same bytes as ISO-8859-8)"),
    ("Iso8859_10", "ISO-8859-10", None, "iso8859_10", True, "Latin 6, Nordic"),
    ("Iso8859_13", "ISO-8859-13", 28603, "iso8859_13", True, "Latin 7, Baltic"),
    ("Iso8859_14", "ISO-8859-14", None, "iso8859_14", True, "Latin 8, Celtic"),
    ("Iso8859_15", "ISO-8859-15", 28605, "iso8859_15", True, "Latin 9, Latin 1 with the euro sign"),
    ("Iso8859_16", "ISO-8859-16", None, "iso8859_16", True, "Latin 10, South-Eastern European"),
    ("Koi8R", "KOI8-R", 20866, "koi8_r", True, "Russian"),
    ("Koi8U", "KOI8-U", 21866, "koi8_u", True, "Ukrainian and Belarusian (WHATWG's KOI8-RU)"),
    ("Macintosh", "macintosh", 10000, "mac_roman", True, "Mac OS Roman"),
    ("Windows874", "windows-874", 874, "cp874", True, "Thai (also TIS-620 and ISO-8859-11)"),
    ("Windows1250", "windows-1250", 1250, "cp1250", True, "Windows Central European"),
    ("Windows1251", "windows-1251", 1251, "cp1251", True, "Windows Cyrillic"),
    ("Windows1252", "windows-1252", 1252, "cp1252", True, "Windows Western (also ISO-8859-1 and ASCII on the web)"),
    ("Windows1253", "windows-1253", 1253, "cp1253", True, "Windows Greek"),
    ("Windows1254", "windows-1254", 1254, "cp1254", True, "Windows Turkish (also ISO-8859-9)"),
    ("Windows1255", "windows-1255", 1255, "cp1255", True, "Windows Hebrew"),
    ("Windows1256", "windows-1256", 1256, "cp1256", True, "Windows Arabic"),
    ("Windows1257", "windows-1257", 1257, "cp1257", True, "Windows Baltic"),
    ("Windows1258", "windows-1258", 1258, "cp1258", True, "Windows Vietnamese"),
    ("XMacCyrillic", "x-mac-cyrillic", 10007, "mac_cyrillic", True, "Mac OS Cyrillic"),
    ("XUserDefined", "x-user-defined", None, None, True, "bytes 0x80..0xFF as U+F780..U+F7FF (WHATWG)"),
    ("Latin1", "ISO-8859-1", 28591, "latin_1", False, "ISO-8859-1 exactly: each byte is its code point (the web reads the label as windows-1252)"),
    ("Ibm437", "IBM437", 437, "cp437", False, "DOS United States (also ZIP names without the UTF-8 flag)"),
    ("Ibm737", "IBM737", 737, "cp737", False, "DOS Greek"),
    ("Ibm775", "IBM775", 775, "cp775", False, "DOS Baltic"),
    ("Ibm850", "IBM850", 850, "cp850", False, "DOS Western European"),
    ("Ibm852", "IBM852", 852, "cp852", False, "DOS Central European"),
    ("Ibm855", "IBM855", 855, "cp855", False, "DOS Cyrillic (IBM)"),
    ("Ibm857", "IBM857", 857, "cp857", False, "DOS Turkish"),
    ("Ibm858", "IBM00858", 858, "cp858", False, "DOS Western European with the euro sign"),
    ("Ibm860", "IBM860", 860, "cp860", False, "DOS Portuguese"),
    ("Ibm861", "IBM861", 861, "cp861", False, "DOS Icelandic"),
    ("Ibm862", "IBM862", 862, "cp862", False, "DOS Hebrew"),
    ("Ibm863", "IBM863", 863, "cp863", False, "DOS Canadian French"),
    ("Ibm864", "IBM864", 864, "cp864", False, "DOS Arabic"),
    ("Ibm865", "IBM865", 865, "cp865", False, "DOS Nordic"),
    ("Ibm869", "IBM869", 869, "cp869", False, "DOS Greek 2"),
]

# code pages that name another set's table: ISO-8859-9 (28599) and TIS-620 / ISO-8859-11 are
# windows-1254 and windows-874 on the web, a superset of the original
CODE_PAGE_ALIASES = {28599: "Windows1254"}

# where the standard's index differs from Python's codec (beyond the C1 rule)
WHATWG_FIXES = {
    "Koi8U": {0xAE: 0x045E, 0xBE: 0x040E},   # KOI8-RU: Belarusian ў and Ў
    "Windows1255": {0xCA: 0x05BA},           # HEBREW POINT HOLAM HASER FOR VAV
}

# the WHATWG labels (encodings.json), by what they name: a variant of CHARSETS, or a Label
LABELS = {
    "Utf8": ["unicode-1-1-utf-8", "unicode11utf8", "unicode20utf8", "utf-8", "utf8", "x-unicode20utf8"],
    "Ibm866": ["866", "cp866", "csibm866", "ibm866"],
    "Iso8859_2": ["csisolatin2", "iso-8859-2", "iso-ir-101", "iso8859-2", "iso88592", "iso_8859-2", "iso_8859-2:1987", "l2", "latin2"],
    "Iso8859_3": ["csisolatin3", "iso-8859-3", "iso-ir-109", "iso8859-3", "iso88593", "iso_8859-3", "iso_8859-3:1988", "l3", "latin3"],
    "Iso8859_4": ["csisolatin4", "iso-8859-4", "iso-ir-110", "iso8859-4", "iso88594", "iso_8859-4", "iso_8859-4:1988", "l4", "latin4"],
    "Iso8859_5": ["csisolatincyrillic", "cyrillic", "iso-8859-5", "iso-ir-144", "iso8859-5", "iso88595", "iso_8859-5", "iso_8859-5:1988"],
    "Iso8859_6": ["arabic", "asmo-708", "csiso88596e", "csiso88596i", "csisolatinarabic", "ecma-114", "iso-8859-6", "iso-8859-6-e",
                  "iso-8859-6-i", "iso-ir-127", "iso8859-6", "iso88596", "iso_8859-6", "iso_8859-6:1987"],
    "Iso8859_7": ["csisolatingreek", "ecma-118", "elot_928", "greek", "greek8", "iso-8859-7", "iso-ir-126", "iso8859-7", "iso88597",
                  "iso_8859-7", "iso_8859-7:1987", "sun_eu_greek"],
    "Iso8859_8": ["csiso88598e", "csisolatinhebrew", "hebrew", "iso-8859-8", "iso-8859-8-e", "iso-ir-138", "iso8859-8", "iso88598",
                  "iso_8859-8", "iso_8859-8:1988", "visual"],
    "Iso8859_8I": ["csiso88598i", "iso-8859-8-i", "logical"],
    "Iso8859_10": ["csisolatin6", "iso-8859-10", "iso-ir-157", "iso8859-10", "iso885910", "l6", "latin6"],
    "Iso8859_13": ["iso-8859-13", "iso8859-13", "iso885913"],
    "Iso8859_14": ["iso-8859-14", "iso8859-14", "iso885914"],
    "Iso8859_15": ["csisolatin9", "iso-8859-15", "iso8859-15", "iso885915", "iso_8859-15", "l9"],
    "Iso8859_16": ["iso-8859-16"],
    "Koi8R": ["cskoi8r", "koi", "koi8", "koi8-r", "koi8_r"],
    "Koi8U": ["koi8-ru", "koi8-u"],
    "Macintosh": ["csmacintosh", "mac", "macintosh", "x-mac-roman"],
    "Windows874": ["dos-874", "iso-8859-11", "iso8859-11", "iso885911", "tis-620", "windows-874"],
    "Windows1250": ["cp1250", "windows-1250", "x-cp1250"],
    "Windows1251": ["cp1251", "windows-1251", "x-cp1251"],
    "Windows1252": ["ansi_x3.4-1968", "ascii", "cp1252", "cp819", "csisolatin1", "ibm819", "iso-8859-1", "iso-ir-100", "iso8859-1",
                    "iso88591", "iso_8859-1", "iso_8859-1:1987", "l1", "latin1", "us-ascii", "windows-1252", "x-cp1252"],
    "Windows1253": ["cp1253", "windows-1253", "x-cp1253"],
    "Windows1254": ["cp1254", "csisolatin5", "iso-8859-9", "iso-ir-148", "iso8859-9", "iso88599", "iso_8859-9", "iso_8859-9:1989", "l5",
                    "latin5", "windows-1254", "x-cp1254"],
    "Windows1255": ["cp1255", "windows-1255", "x-cp1255"],
    "Windows1256": ["cp1256", "windows-1256", "x-cp1256"],
    "Windows1257": ["cp1257", "windows-1257", "x-cp1257"],
    "Windows1258": ["cp1258", "windows-1258", "x-cp1258"],
    "XMacCyrillic": ["x-mac-cyrillic", "x-mac-ukrainian"],
    "XUserDefined": ["x-user-defined"],
    "Other(\"GBK\")": ["chinese", "csgb2312", "csiso58gb231280", "gb2312", "gb_2312", "gb_2312-80", "gbk", "iso-ir-58", "x-gbk"],
    "Other(\"gb18030\")": ["gb18030"],
    "Other(\"Big5\")": ["big5", "big5-hkscs", "cn-big5", "csbig5", "x-x-big5"],
    "Other(\"EUC-JP\")": ["cseucpkdfmtjapanese", "euc-jp", "x-euc-jp"],
    "Other(\"ISO-2022-JP\")": ["csiso2022jp", "iso-2022-jp"],
    "Other(\"Shift_JIS\")": ["csshiftjis", "ms932", "ms_kanji", "shift-jis", "shift_jis", "sjis", "windows-31j", "x-sjis"],
    "Other(\"EUC-KR\")": ["cseuckr", "csksc56011987", "euc-kr", "iso-ir-149", "korean", "ks_c_5601-1987", "ks_c_5601-1989", "ksc5601",
                          "ksc_5601", "windows-949"],
    "Replacement": ["csiso2022kr", "hz-gb-2312", "iso-2022-cn", "iso-2022-cn-ext", "iso-2022-kr", "replacement"],
    "Utf16Be": ["unicodefffe", "utf-16be"],
    "Utf16Le": ["csunicode", "iso-10646-ucs-2", "ucs-2", "unicode", "unicodefeff", "utf-16", "utf-16le"],
}
# the WHATWG name of each Label that is not a character set (for --verify against Node)
LABEL_NAMES = {"Utf8": "utf-8", "Utf16Be": "utf-16be", "Utf16Le": "utf-16le", "Replacement": "replacement"}


def table(variant, codec, whatwg):
    if variant == "XUserDefined":
        return [0xF780 + i for i in range(128)]
    out = []
    for b in range(0x80, 0x100):
        try:
            c = ord(bytes([b]).decode(codec))
        except UnicodeDecodeError:
            c = b if whatwg and b < 0xA0 else 0
        out.append(c)
    for b, c in WHATWG_FIXES.get(variant, {}).items():
        out[b - 0x80] = c
    return out


def label_target(key):
    return f"Label::Single(Charset::{key})" if key in {c[0] for c in CHARSETS} else f"Label::{key}"


def render():
    L = [
        "// charsets -- the single-byte character sets of formats::encoding (generated by tools/gen_encodings.py,",
        "// do not edit): the WHATWG Encoding Standard's single-byte encodings and labels, the DOS code pages of old",
        "// files.  Tables: bytes 0x80..0xFF -> Unicode, 0 unassigned (below 0x80 every set is ASCII).  The labels and",
        "// the standard's tables: Copyright © WHATWG (Apple, Google, Mozilla, Microsoft), BSD 3-Clause License",
        "// (licenses/WHATWG-LICENSE.txt).",
        "",
        "use crate::encoding::Label;",
        "",
        "/// a single-byte character set: the bytes below 0x80 are ASCII, the others come from its table.",
        "/// The single-byte encodings of the WHATWG Encoding Standard (what browsers, HTML and mail use; `whatwg()`)",
        "/// and the DOS code pages of old files (by `from_code_page` or by name here; no web label names them)",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]",
        "#[non_exhaustive]",
        "pub enum Charset {",
    ]
    for v, name, cp, _, w, desc in CHARSETS:
        L.append(f"    /// {name}: {desc}{'' if w else ' (not WHATWG)'}")
        L.append(f"    {v},")
    L += ["}", "", "impl Charset {", "    /// every character set, in declaration order", "    pub const ALL: &'static [Charset] = &["]
    for i in range(0, len(CHARSETS), 6):
        L.append("        " + " ".join(f"Charset::{c[0]}," for c in CHARSETS[i:i + 6]))
    L += ["    ];", "", "    /// its name: the WHATWG Encoding Standard's, else IANA's (\"IBM437\", \"ISO-8859-1\")",
          "    pub const fn name(self) -> &'static str {", "        match self {"]
    L += [f"            Charset::{v} => \"{name}\"," for v, name, *_ in CHARSETS]
    L += ["        }", "    }", "", "    /// whether it is one of the WHATWG Encoding Standard's encodings (the others have no web label)",
          "    pub const fn whatwg(self) -> bool {", "        match self {"]
    yes = " | ".join(f"Charset::{c[0]}" for c in CHARSETS if c[4])
    L += [f"            {yes} => true,", "            _ => false,", "        }", "    }", "",
          "    /// its Windows code page number (None: ISO-8859-10, -14 and -16 and x-user-defined have none)",
          "    pub const fn code_page(self) -> Option<u16> {", "        match self {"]
    L += [f"            Charset::{v} => {'Some(' + str(cp) + ')' if cp else 'None'}," for v, _, cp, *_ in CHARSETS]
    L += ["        }", "    }", "",
          "    /// the character set of a Windows code page number: 437 to 869 (DOS), 874, 1250 to 1258, 10000 and 10007",
          "    /// (Mac), 20866 and 21866 (KOI8), 28591 to 28605 (ISO-8859; 28599, ISO-8859-9, is windows-1254 as on the",
          "    /// web), 38598",
          "    pub const fn from_code_page(cp: u16) -> Option<Charset> {", "        match cp {"]
    L += [f"            {cp} => Some(Charset::{v})," for v, _, cp, *_ in sorted(CHARSETS, key=lambda c: c[2] or 0) if cp]
    L += [f"            {cp} => Some(Charset::{v})," for cp, v in sorted(CODE_PAGE_ALIASES.items())]
    L += ["            _ => None,", "        }", "    }", "",
          "    // bytes 0x80..0xFF -> code points (0: unassigned)",
          "    pub(crate) fn table(self) -> &'static [u16; 128] {", "        match self {"]
    L += [f"            Charset::{v} => &{v.upper()}," for v, *_ in CHARSETS]
    L += ["        }", "    }", "}", "",
          "// a label of the WHATWG Encoding Standard, already without whitespace and in lower case -> what it names",
          "pub(crate) fn lookup(label: &str) -> Option<Label> {", "    Some(match label {"]
    for key, labels in LABELS.items():
        pats = " | ".join(f'"{l}"' for l in labels)
        L.append(f"        {pats} => {label_target(key)},")
    L += ["        _ => return None,", "    })", "}", ""]
    for v, name, _, codec, w, _ in CHARSETS:
        t = table(v, codec, w)
        L.append(f"// {name}")
        L.append(f"static {v.upper()}: [u16; 128] = [")
        for r in range(0, 128, 16):
            L.append("    " + ", ".join(f"0x{x:04X}" for x in t[r:r + 16]) + ",")
        L.append("];")
    return "\n".join(L) + "\n"


# ---------------------------------------------------------------- --verify
def encoding_rs():
    """encoding_rs's data.rs and lib.rs from Cargo's registry, or None"""
    reg = Path.home() / ".cargo" / "registry" / "src"
    found = sorted(reg.glob("*/encoding_rs-*/src/data.rs")) if reg.is_dir() else []
    return found[-1].parent if found else None


# encoding_rs's field and constant names of the WHATWG sets
RS_FIELD = {"Ibm866": "ibm866", "Koi8R": "koi8_r", "Koi8U": "koi8_u", "Macintosh": "macintosh", "XMacCyrillic": "x_mac_cyrillic"}


def rs_field(v):
    if v in RS_FIELD:
        return RS_FIELD[v]
    if v.startswith("Iso8859_"):
        return "iso_8859_" + v[len("Iso8859_"):].lower().replace("8i", "8")
    if v.startswith("Windows"):
        return "windows_" + v[len("Windows"):]
    return None


def verify():
    bad = []
    by_name = {c[0]: c for c in CHARSETS}
    src = encoding_rs()
    if src is None:
        print("verify: encoding_rs is not in Cargo's registry: the WHATWG data were not compared")
    else:
        data = (src / "data.rs").read_text(encoding="utf-8")
        body = data[data.index("pub static SINGLE_BYTE_DATA"):]
        rs = {m.group(1): [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]+)", m.group(2))]
              for m in re.finditer(r"(\w+): \[\s*([^\]]+)\]", body)}
        n = 0
        for v, _, _, codec, w, _ in CHARSETS:
            f = rs_field(v) if w else None
            if f is None:
                continue
            want = rs.get(f)
            got = table(v, codec, w)
            if want != got:
                bad.append(f"{v}: differs from the WHATWG index at {[hex(0x80 + i) for i in range(128) if want and want[i] != got[i]]}")
            n += 1
        lib = (src / "lib.rs").read_text(encoding="utf-8")
        a = lib[lib.index("static LABELS_SORTED"):]
        b = lib[lib.index("static ENCODINGS_IN_LABEL_SORT"):]
        rs_labels = dict(zip(re.findall(r'"([^"]*)"', a[:a.index("];")]), re.findall(r"&(\w+)_INIT", b[:b.index("];")])))
        mine = {l: k for k, ls in LABELS.items() for l in ls}
        if set(rs_labels) != set(mine):
            bad.append(f"labels: missing {sorted(set(rs_labels) - set(mine))}, extra {sorted(set(mine) - set(rs_labels))}")
        groups = {}
        for l, e in rs_labels.items():
            groups.setdefault(e, set()).add(l)
        mine_groups = {frozenset(ls) for ls in LABELS.values()}
        if {frozenset(g) for g in groups.values()} != mine_groups:
            bad.append("labels: grouped differently from the WHATWG data")
        print(f"verify: {n} tables and {len(rs_labels)} labels against the WHATWG data of {src.parent.name}")
    if shutil.which("node"):
        names = {k: by_name[k][1] if k in by_name else LABEL_NAMES.get(k) for k in LABELS}
        js = ("const L=%s,T=%s,out={labels:{},tables:{}};for(const [l,n] of L){try{out.labels[l]=new TextDecoder(l).encoding}catch(e){out.labels[l]=null}}"
              "for(const n of T){let d;try{d=new TextDecoder(n)}catch(e){continue}out.tables[n]=[];for(let b=128;b<256;b++){const s=d.decode(new Uint8Array([b]));"
              "out.tables[n].push(s.codePointAt(0))}}console.log(JSON.stringify(out))")
        pairs = [(l, names[k]) for k, ls in LABELS.items() if names[k] and not k.startswith("Other") for l in ls]
        node = json.loads(subprocess.run(["node", "-e", js % (json.dumps(pairs), json.dumps([c[1] for c in CHARSETS if c[4]]))],
                                         capture_output=True, text=True, check=True).stdout)
        # what ICU (Node) does other than the standard, known and expected
        icu = {("windows-874", 0xDB), ("windows-874", 0xDC), ("windows-874", 0xDD), ("windows-874", 0xDE), ("windows-874", 0xFC),
               ("windows-874", 0xFD), ("windows-874", 0xFE), ("windows-874", 0xFF), ("windows-1253", 0xAA), ("windows-1255", 0xCA),
               ("KOI8-U", 0xAE), ("KOI8-U", 0xBE)}
        seen_icu, labels_ok, tables_ok, missing = set(), 0, 0, []
        for l, want in pairs:
            got = node["labels"].get(l)
            if got is None:
                missing.append(l)
            elif got.lower() == want.lower():
                labels_ok += 1
            else:
                bad.append(f"label {l}: Node names {got}, the table here {want}")
        for v, name, _, codec, w, _ in CHARSETS:
            if not w or name not in node["tables"]:
                continue
            got = table(v, codec, w)
            nt = [0 if c == 0xFFFD else c for c in node["tables"][name]]
            diff = {(name, 0x80 + i) for i in range(128) if nt[i] != got[i]}
            seen_icu |= diff & icu
            if diff - icu:
                bad.append(f"{name}: differs from Node at {sorted(hex(b) for _, b in diff - icu)}")
            else:
                tables_ok += 1
        print(f"verify: Node {subprocess.run(['node', '--version'], capture_output=True, text=True).stdout.strip()}: {labels_ok} labels and"
              f" {tables_ok} tables agree; ICU's known differences seen: {len(seen_icu)} of {len(icu)}; labels Node lacks: {missing}")
    else:
        print("verify: no node on PATH: TextDecoder was not compared")
    for b in bad:
        print("FAIL", b)
    return 1 if bad else 0


def main():
    if "--verify" in sys.argv:
        return verify()
    text = render()
    if "--check" in sys.argv:
        ok = OUT.exists() and OUT.read_text(encoding="utf-8") == text
        print("charsets.rs is current" if ok else "charsets.rs is stale: python tools/gen_encodings.py")
        return 0 if ok else 1
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
