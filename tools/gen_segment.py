#!/usr/bin/env python3
"""gen_segment.py -- the property table of lib/text/segment.rs (lib/text/segment_data.rs)

Unicode text segmentation needs Grapheme_Cluster_Break, Word_Break, Line_Break and
Extended_Pictographic, which Python's unicodedata does not have.  They are read from Perl's
copy of the Unicode database (unicore/To/GCB.pl, WB.pl, Lb.pl, made by Perl's mktables from
the UCD); General_Category and East_Asian_Width come from Python's unicodedata.

  python tools/gen_segment.py [--unicore DIR] [--out lib/text/segment_data.rs]

DIR defaults to Git for Windows' Perl (C:/Program Files/Git/usr/share/perl5/core_perl/unicore)
or, on Linux, the first /usr/share/perl/*/unicore.  The output is two arrays: the first code
point of every run of equal properties and the packed properties of the run (see segment.rs).
"""
import argparse
import glob
import os
import re
import sys
import unicodedata

GCB = ["Other", "CR", "LF", "Control", "Extend", "ZWJ", "Regional_Indicator", "Prepend", "SpacingMark",
       "L", "V", "T", "LV", "LVT"]
WB = ["Other", "CR", "LF", "Newline", "Extend", "ZWJ", "Regional_Indicator", "Format", "Katakana",
      "Hebrew_Letter", "ALetter", "Single_Quote", "Double_Quote", "MidNumLet", "MidLetter", "MidNum",
      "Numeric", "ExtendNumLet", "WSegSpace"]
LB = ["XX", "BK", "CR", "LF", "CM", "NL", "SG", "WJ", "ZW", "GL", "SP", "ZWJ", "B2", "BA", "BB", "HY",
      "CB", "CL", "CP", "EX", "IN", "NS", "OP", "QU", "IS", "NU", "PO", "PR", "SY", "AI", "AL", "CJ",
      "EB", "EM", "H2", "H3", "HL", "ID", "JL", "JV", "JT", "RI", "SA"]
MAXCP = 0x110000

# Line_Break defaults of unassigned code points (UAX #14, DerivedLineBreak @missing)
LB_DEFAULTS = [
    (0x3400, 0x4DBF, "ID"), (0x4E00, 0x9FFF, "ID"), (0xF900, 0xFAFF, "ID"), (0x20000, 0x2FFFD, "ID"),
    (0x30000, 0x3FFFD, "ID"), (0x1F000, 0x1FAFF, "ID"), (0x1FC00, 0x1FFFD, "ID"), (0x20A0, 0x20CF, "PR"),
]


def default_unicore():
    cands = [r"C:/Program Files/Git/usr/share/perl5/core_perl/unicore"]
    cands += sorted(glob.glob("/usr/share/perl/*/unicore"), reverse=True)
    cands += ["/usr/share/perl5/core_perl/unicore", "/usr/lib/perl5/core_perl/unicore"]
    for c in cands:
        if os.path.exists(os.path.join(c, "To", "GCB.pl")):
            return c
    sys.exit("gen_segment: no Perl unicore folder found (--unicore DIR)")


def read_map(path):
    """the ranges of a mktables To/*.pl file: [(first, last, value)]"""
    out = []
    body = False
    for line in open(path, encoding="utf-8"):
        if line.startswith("return <<'END'"):
            body = True
            continue
        if not body or line.startswith("END"):
            continue
        parts = line.rstrip("\n").split("\t")
        if len(parts) < 3 or not re.match(r"^[0-9A-F]+$", parts[0]):
            continue
        a = int(parts[0], 16)
        b = int(parts[1], 16) if parts[1] else a
        out.append((a, b, parts[2]))
    return out


def fill(ranges, default, size=MAXCP):
    arr = [default] * size
    for a, b, v in ranges:
        for cp in range(a, b + 1):
            arr[cp] = v
    return arr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--unicore", default=None)
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib", "text", "segment_data.rs"))
    a = ap.parse_args()
    uc = a.unicore or default_unicore()
    version = open(os.path.join(uc, "version")).read().strip()

    gcb = fill(read_map(os.path.join(uc, "To", "GCB.pl")), "Other")
    wb = fill(read_map(os.path.join(uc, "To", "WB.pl")), "Other")
    lb = fill(LB_DEFAULTS, "XX")
    for x, y, v in read_map(os.path.join(uc, "To", "Lb.pl")):
        for cp in range(x, y + 1):
            lb[cp] = v

    props = []
    for cp in range(MAXCP):
        g = gcb[cp]
        ext = False
        if g.startswith("ExtPict"):
            ext = True
            g = "Other"
        w = wb[cp]
        if w.startswith("ExtPict"):
            ext = True
            w = "ALetter" if w.endswith("_LE") else "Other"
        if w == "Perl_Tailored_HSpace":
            # Perl's \h: UAX #29's WSegSpace is the spaces of Zs without the no-break ones
            ch = chr(cp)
            w = "WSegSpace" if unicodedata.category(ch) == "Zs" and cp not in (0xA0, 0x2007, 0x202F) else "Other"
        l = lb[cp]
        gc = unicodedata.category(chr(cp)) if cp < 0xD800 or cp > 0xDFFF else "Cs"
        # LB1: AI, SG, XX -> AL; SA -> CM (Mn, Mc) or AL; CJ -> NS
        if l in ("AI", "SG", "XX"):
            l = "AL"
        elif l == "SA":
            l = "CM" if gc in ("Mn", "Mc") else "AL"
        elif l == "CJ":
            l = "NS"
        wide = unicodedata.east_asian_width(chr(cp)) in ("F", "W", "H") if gc != "Cs" else False
        alnum = gc[0] in ("L", "N")
        space = gc == "Zs" or cp in (0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x85, 0x2028, 0x2029)
        unassigned = gc == "Cn"
        if g not in GCB or w not in WB or l not in LB:
            sys.exit(f"gen_segment: unknown value at U+{cp:04X}: {g} {w} {l}")
        p = GCB.index(g) | WB.index(w) << 4 | LB.index(l) << 9 | ext << 15 | wide << 16 | alnum << 17 | space << 18 | unassigned << 19
        props.append(p)

    starts, vals = [], []
    for cp, p in enumerate(props):
        if not vals or vals[-1] != p:
            starts.append(cp)
            vals.append(p)

    def arr(name, ty, xs):
        lines = []
        for k in range(0, len(xs), 12):
            lines.append("    " + ", ".join(f"0x{x:X}" for x in xs[k:k + 12]) + ",")
        return f"pub(crate) static {name}: [{ty}; {len(xs)}] = [\n" + "\n".join(lines) + "\n];\n"

    out = (
        "// segment_data -- generated by tools/gen_segment.py from Perl's copy of the Unicode database\n"
        f"// (version {version}) and Python's unicodedata {unicodedata.unidata_version}; do not edit\n"
        "//\n"
        "// RUN_START[i]: the first code point of run i; RUN_PROPS[i]: its properties, packed: bits 0..3\n"
        "// Grapheme_Cluster_Break, 4..8 Word_Break, 9..14 Line_Break (after LB1), 15\n"
        "// Extended_Pictographic, 16 East_Asian_Width F / W / H, 17 a letter or number (L*, N*), 18 white\n"
        "// space, 19 unassigned (segment.rs has the value lists)\n\n"
        f"pub(crate) const UNICODE_VERSION: &str = \"{version}\";\n\n"
        + arr("RUN_START", "u32", starts) + "\n" + arr("RUN_PROPS", "u32", vals)
    )
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        f.write(out)
    print(f"gen_segment: {len(starts)} runs from {uc} (Unicode {version}) -> {a.out}")


if __name__ == "__main__":
    main()
