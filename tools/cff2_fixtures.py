#!/usr/bin/env python3
"""cff2_fixtures.py -- synthetic CFF2 variable fonts and CFF comparison fonts for
`fonttest --selftest` and the font dialog (apps/fonttest/tests/cff2, cff2ref,
apps/fontpicktest/tests/var_cff2.uis).

Writes apps/fonttest/tests/cff2/ (only CFF2 faces: the fallback check scans this folder):
  jfcff2.otf   "jfgui CFF2": axes wght 100..400..900 and wdth 50..100..100, four named
               instances (German and English names), 9 glyphs in two font DICTs (FDSelect
               format 3), blend in charstrings (up to 79 operands: more than CFF's 48) and in
               both Private DICTs (BlueValues, StdHW, a real BlueScale), two item variation
               data (k = 5 and 2 regions: wght+, wdth-, a corner region, an intermediate
               region, wght-), vsindex in a Private DICT and in a charstring, local subrs of
               each font DICT and global subrs (blend inside), hint operators with blended
               stems, hintmask / cntrmask (also inside a subr), every path operator incl. the
               four flex forms, a 16.16 fixed default, an empty glyph; HVAR advances
  jfcff2.ttc   an OpenType collection of two faces "jfgui CFF2 JP" / "jfgui CFF2 SC" that
               share one CFF2 table (and hmtx, HVAR, fvar, ...; only name and cmap differ:
               U+4E2D maps to glyph 6 / glyph 7) with other encodings of the same outlines:
               FDSelect format 4, the font DICTs swapped, 1240 global subrs (bias 1131),
               maxstack 256, no explicit vsindex 0
  limits.bin   (not a font extension: font scans skip it) one glyph per limit: maxstack 300
               (250 operands work, 301 do not), blends with too few operands or a negative
               count, a missing item variation data, vsindex after blend, a subr calling
               itself, 600 regions (more than 512), hintmask bytes past the end, a Private
               DICT whose blend lacks operands (its font DICT has no Subrs then)
  expected.txt what fonttest --selftest prints for these fonts at several instances,
               computed here from the charstrings (independent of the Rust code)
Writes apps/fonttest/tests/cff2ref/ (CFF 1 fonts, the outlines the generator computed):
  ref-default.otf, ref-w650.otf, ref-cb.otf, ref-w250.otf   "jfgui CFF2 Ref Default / 650 /
               CB / 250": the outlines and advances of jfcff2.otf at its default instance, at
               wght=650, at wght=900,wdth=75 (the named instance "Condensed Black") and at
               wght=250; every coordinate is exact in f32 and 16.16 (region scalars 0, 0.5, 1),
               so the CFF2 font at these instances must rasterize byte for byte like them

Only synthetic geometry: the files may be redistributed freely.
  python tools/cff2_fixtures.py
"""
import math
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from font_fixtures import (checksum, cff_index, cff_int, cmap4, cmap12, dict_i32, fmt_outline,  # noqa: E402
                           head_table, hhea_table, name_table, os2_table, post_table, sfnt)
from var_fixtures import fvar_table, hvar_table, ivs, normalize  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "apps" / "fonttest" / "tests" / "cff2"
REF = ROOT / "apps" / "fonttest" / "tests" / "cff2ref"
F2 = 16384

# ------------------------------------------------------------------ axes, regions, instances
AXES = [("wght", 100, 400, 900, 0, 256), ("wdth", 50, 100, 100, 0, 257)]
INSTANCES = [(260, [250, 100]), (261, [400, 100]), (262, [650, 100]), (263, [900, 75])]
# regions (start, peak, end) per axis: R0 wght+, R1 wdth-, R2 the corner wght+ wdth-, R3 an
# intermediate wght region (peak 0.5), R4 wght-
REGIONS = [[(0, 1, 1), (0, 0, 0)], [(0, 0, 0), (-1, -1, 0)], [(0, 1, 1), (-1, -1, 0)],
           [(0, 0.5, 1), (0, 0, 0)], [(-1, -1, 0), (0, 0, 0)]]
IVDS = [[0, 1, 2, 3, 4], [0, 4]]                 # item variation data 0: k = 5, 1: k = 2
# the comparison instances (region scalars 0, 0.5, 1) and two with other scalars (expected.txt
# only; one axis, so every value stays a multiple of 2^-14: exact in f32 as in double)
REFS = [("default", {}, "Default", 400), ("w650", {"wght": 650}, "650", 650),
        ("cb", {"wght": 900, "wdth": 75}, "CB", 900), ("w250", {"wght": 250}, "250", 250)]
ODDS = [{"wght": 777}, {"wght": 160}]

Z5 = [0, 0, 0, 0, 0]
Z2 = [0, 0]


def B(*vals):
    """a blend of len(vals) values (default, [delta per region]) -> charstring tokens"""
    out = [v for v, _ in vals]
    for _, d in vals:
        out += d
    return out + [len(vals), "blend"]


def LS(i):
    return ("lsubr", i)


def GS(i):
    return ("gsubr", i)


# ------------------------------------------------------------------ the glyphs (CFF2 charstrings)
# font DICT 0 (Latin, vsindex 0: k = 5), font DICT 1 (Han, Private vsindex 1: k = 2)
G_GLYPHS = [
    # 0 .notdef: a box with a hole, the width blended
    (0, [50, 0, "rmoveto", *B((400, [100, 0, 0, 0, -60])), "hlineto", 700, "vlineto",
         *B((-400, [-100, 0, 0, 0, 60])), "hlineto", *B((50, [20, 0, 0, 0, -10])), -650, "rmoveto",
         600, "vlineto", *B((300, [60, 0, 0, 0, -40])), "hlineto", -600, "vlineto"]),
    # 1 space: an empty charstring
    (0, []),
    # 2 A: blended hstem / vstem, rlineto with 4 blended values, a hole
    (0, [*B((0, Z5), (60, [20, 0, 0, 0, -10])), "hstem", *B((300, Z5), (80, [20, -10, 0, 0, -10])), "vstem",
         *B((100, [-30, 10, 0, 0, 20]), (0, Z5)), "rmoveto",
         *B((350, [40, -50, -20, 0, -30]), (700, Z5), (350, [40, -50, -20, 0, -30]), (-700, Z5)), "rlineto",
         *B((-450, [-40, 50, 20, 0, 30]), (150, [0, 0, 0, 10, 0])), "rmoveto",
         *B((200, [0, -20, 0, 0, 10])), "hlineto",
         *B((-100, [0, 10, 0, 0, -5]), (250, [0, 0, 0, -10, 0])), "rlineto"]),
    # 3 O: vhcurveto with 8 blended values (49 operands), its second half in global subr 0, a
    # hole of rrcurveto with a 16.16 default
    (0, [*B((100, [-20, 30, 0, 0, 20]), (350, Z5)), "rmoveto",
         *B((193, Z5), (157, [10, -10, 0, 0, -10]), (157, Z5), (193, Z5), (193, Z5), (157, [10, -10, 0, 0, -10]),
            (-157, Z5), (-193, Z5)), "vhcurveto", GS(0),
         *B((150, [20, -10, 0, 0, -10]), (0, Z5)), "rmoveto",
         0, -110, 89.5, -90, 110, 0, "rrcurveto", 110, 0, 90, 90, 0, 110, "rrcurveto",
         0, 110, -90, 90, -110, 0, "rrcurveto", -110, 0, -90, -90, 0, -110, "rrcurveto"]),
    # 4 B: blended hstemhm / vstemhm, hintmask, local subr 0, hvcurveto with 8 blended values,
    # global subr 1 (rcurveline), a second hintmask, rlinecurve; a hole of vvcurveto / hhcurveto
    (0, [*B((0, Z5), (50, [10, 0, 0, 0, -10]), (300, Z5), (50, [10, 0, 0, 0, -10]), (250, Z5), (50, Z5)), "hstemhm",
         *B((100, Z5), (60, [30, -10, 0, 0, -20])), "vstemhm", "hintmask", "mask:F0",
         *B((100, [-10, 0, 0, 0, 10]), (0, Z5)), "rmoveto", LS(0),
         *B((250, [30, -30, 0, 0, -20])), "hlineto",
         *B((100, [10, -10, 0, 0, -10]), (50, Z5), (-50, Z5), (-125, Z5), (-100, Z5), (-50, Z5), (-75, Z5),
            (-100, [-10, 10, 0, 0, 10])), "hvcurveto", GS(1), "hintmask", "mask:30",
         -10, -10, -60, -80, -80, -40, -150, 0, "rlinecurve", *B((-150, [-20, 30, 0, 0, 20])), "hlineto",
         100, 450, "rmoveto", 100, "hlineto", 40, 30, 10, 40, 30, "vvcurveto", 20, -30, -10, 20, -20,
         "hhcurveto", -90, "hlineto"]),
    # 5 H: vsindex 1 in the charstring (k = 2), one vlineto with 11 blended values
    (0, [1, "vsindex", *B((100, [-20, 10]), (0, Z2)), "rmoveto",
         *B((700, Z2), (120, [40, -30]), (-270, Z2), (260, [-40, 20]), (270, Z2), (120, [40, -30]), (-700, Z2),
            (-120, [-40, 30]), (270, Z2), (-260, [40, -20]), (-270, Z2)), "vlineto"]),
    # 6 U+4E2D (JP form): font DICT 1 (vsindex 1 from its Private DICT), blended stems,
    # hintmask, local subr 0 of font DICT 1 (the counter), local subr 1 (the stroke, a cntrmask
    # inside the subr)
    (1, [*B((180, Z2), (120, [30, -20]), (200, Z2), (120, [30, -20])), "hstemhm", *B((440, [-10, 5]), (120, [20, -10])),
         "vstemhm", "hintmask", "mask:E0", 120, 180, "rmoveto",
         *B((760, [60, -40]), (440, Z2), (-760, [-60, 40])), "hlineto", *B((120, [20, -10]), (-120, Z2)), "rmoveto",
         LS(0), *B((200, [-10, 5]), (-380, Z2)), "rmoveto", LS(1)]),
    # 7 U+4E2D (SC form): a longer stroke drawn inline, global subr 1 from font DICT 1's glyph
    (1, [120, 180, "rmoveto", *B((760, [60, -40]), (440, Z2), (-760, [-60, 40])), "hlineto",
         *B((120, [20, -10]), (-120, Z2)), "rmoveto", LS(0), *B((200, [-10, 5]), (-400, Z2)), "rmoveto",
         *B((120, [20, -10]), (940, Z2), (-120, [-20, 10])), "hlineto", 260, -100, "rmoveto", GS(1)]),
    # 8 V: flex with 13 blended values (79 operands), hflex, hflex1; a dome of flex1
    (0, [*B((100, [-20, 0, 0, 0, 10]), (700, Z5)), "rmoveto",
         *B((60, [0, -5, 0, 0, 5]), (-120, Z5), (60, [2, 0, 0, 0, 0]), (-120, Z5), (55, Z5), (-110, Z5), (55, Z5),
            (-110, Z5), (60, [2, 0, 0, 0, 0]), (-120, Z5), (60, [0, -5, 0, 0, 5]), (-120, Z5), (50, Z5)), "flex",
         30, 30, 20, 30, 30, 30, 30, "hflex", *B((170, [10, -10, 0, 0, -10]), (700, Z5)), "rlineto",
         -30, 10, -30, 10, -30, -30, -30, -10, -30, "hflex1",
         *B((-170, [-10, 10, 0, 0, 10]), (-480, Z5)), "rlineto", -170, 480, "rlineto",
         140, 60, "rmoveto", 10, 20, 20, 20, 20, 0, 20, 0, 20, -20, 10, "flex1"]),
]
# local subrs per font DICT, global subrs
L_SUBRS = [
    [[*B((0, Z5), (700, [0, 0, 0, 10, 0])), "rlineto"]],
    [[*B((520, [-40, 30]), (-200, Z2), (-520, [40, -30])), "hlineto"],
     ["cntrmask", "mask:E0", *B((120, [20, -10]), (900, Z2), (-120, [-20, 10])), "hlineto"]],
]
G_SUBRS = [
    [*B((-193, Z5), (-157, [-10, 10, 0, 0, 10]), (-157, Z5), (-193, Z5), (-193, Z5), (-157, [-10, 10, 0, 0, 10]),
        (157, Z5), (193, Z5)), "vhcurveto"],
    [120, 0, 80, -80, 0, -120, 0, -20, "rcurveline"],
]
ADV = [600, 250, 900, 900, 650, 700, 1000, 1000, 900]
# HVAR deltas per glyph and region (even: whole advances at the comparison instances)
HV_ROWS = [[100, 0, 0, 0, -60], [20, -20, 0, 0, -10], [40, -40, -20, 20, -30], [20, -20, 0, 0, -20],
           [30, -20, 0, 0, -20], [80, 0, 0, 0, -60], [60, -40, 0, 0, -40], [60, -40, 0, 0, -40],
           [20, -20, 0, 0, -20]]
# Private DICT values: BlueValues (6), StdHW, BlueScale
BLUES = [(-10, [0, 0, 0, 0, 0]), (10, [0, 0, 0, 0, 0]), (490, [0, 0, 0, 0, 0]), (10, [0, 0, 0, 0, 0]),
         (190, [10, 0, 0, 0, -10]), (10, [0, 0, 0, 0, 0])]
LETTERS = list(range(0x30, 0x3A)) + list(range(0x41, 0x5B)) + list(range(0x61, 0x7B))
CMAP = {0x20: 1, 0x41: 2, 0x4F: 3, 0x42: 4, 0x48: 5, 0x4E2D: 6, 0x56: 8}
for _i, _c in enumerate(LETTERS):
    CMAP.setdefault(_c, (2, 3, 4, 5, 8)[_i % 5])
NAMES = [(1, 0x409, "jfgui CFF2"), (2, 0x409, "Regular"), (2, 0x407, "Standard"), (4, 0x409, "jfgui CFF2 Regular"),
         (256, 0x409, "Weight"), (256, 0x407, "Strichstärke"), (257, 0x409, "Width"), (257, 0x407, "Breite"),
         (260, 0x409, "Light"), (260, 0x407, "Leicht"), (261, 0x409, "Regular"), (261, 0x407, "Standard"),
         (262, 0x409, "Semibold"), (262, 0x407, "Halbfett"), (263, 0x409, "Condensed Black")]


# ------------------------------------------------------------------ encoders
def subr_bias(n):
    return 107 if n < 1240 else (1131 if n < 33900 else 32768)


def cs2(tokens, lbias=107, gbias=107):
    """a charstring from ints / floats (16.16) / operator names / 'mask:XX' / subr calls"""
    ops = {"hstem": b"\x01", "vstem": b"\x03", "vmoveto": b"\x04", "rlineto": b"\x05", "hlineto": b"\x06",
           "vlineto": b"\x07", "rrcurveto": b"\x08", "callsubr": b"\x0a", "endchar": b"\x0e", "vsindex": b"\x0f",
           "blend": b"\x10", "hstemhm": b"\x12", "hintmask": b"\x13", "cntrmask": b"\x14", "rmoveto": b"\x15",
           "hmoveto": b"\x16", "vstemhm": b"\x17", "rcurveline": b"\x18", "rlinecurve": b"\x19",
           "vvcurveto": b"\x1a", "hhcurveto": b"\x1b", "callgsubr": b"\x1d", "vhcurveto": b"\x1e",
           "hvcurveto": b"\x1f", "hflex": b"\x0c\x22", "flex": b"\x0c\x23", "hflex1": b"\x0c\x24",
           "flex1": b"\x0c\x25"}
    out = b""
    for it in tokens:
        if isinstance(it, tuple):
            kind, i = it
            out += cff_int(i - (lbias if kind == "lsubr" else gbias)) + ops["callsubr" if kind == "lsubr" else "callgsubr"]
        elif isinstance(it, bool):
            raise TypeError(it)
        elif isinstance(it, int):
            out += cff_int(it)
        elif isinstance(it, float):
            out += b"\xff" + struct.pack(">i", int(round(it * 65536)))
        elif it.startswith("mask:"):
            out += bytes.fromhex(it[5:])
        else:
            out += ops[it]
    return out


def index2(items):
    """a CFF2 INDEX (u32 count; an empty one is the count alone)"""
    if not items:
        return struct.pack(">I", 0)
    offs = [1]
    for it in items:
        offs.append(offs[-1] + len(it))
    osz = 1 if offs[-1] < 256 else (2 if offs[-1] < 65536 else (3 if offs[-1] < 1 << 24 else 4))
    return (struct.pack(">IB", len(items), osz) + b"".join(o.to_bytes(osz, "big") for o in offs) +
            b"".join(items))


def dreal(s):
    """a DICT real number from its decimal text"""
    nib = []
    for ch in s:
        nib.append({".": 0xA, "-": 0xE}.get(ch, None) if not ch.isdigit() else int(ch))
    nib.append(0xF)
    if len(nib) % 2:
        nib.append(0xF)
    return b"\x1e" + bytes(nib[i] << 4 | nib[i + 1] for i in range(0, len(nib), 2))


def dblend(vals):
    """DICT operands of a blend of (default, [deltas]) values"""
    b = b"".join(cff_int(v) for v, _ in vals)
    for _, d in vals:
        b += b"".join(cff_int(x) for x in d)
    return b + cff_int(len(vals)) + b"\x17"


def private2(fd, vsindex_op, subrs_rel):
    """a CFF2 Private DICT: [vsindex], BlueValues and StdHW through blend, BlueScale (a real),
    Subrs (offset from the DICT's start)"""
    k = len(IVDS[fd])
    blues = [(v, (d + [0] * k)[:k]) for v, d in BLUES]
    d = b""
    if vsindex_op:
        d += cff_int(fd) + b"\x16"
    d += dblend(blues) + b"\x06"
    d += dblend([(50, ([20, 0, 0, 0, -10] if k == 5 else [20, -10]))]) + b"\x0a"
    d += dreal("0.039625") + b"\x0c\x09"
    d += dict_i32(subrs_rel) + b"\x13"
    return d


def ivs_cff2(ivds=None):
    """the CFF2 VariationStore: {u16 length, an item variation store whose data have no rows}"""
    ivds = IVDS if ivds is None else ivds
    rl = struct.pack(">HH", 2, len(REGIONS)) + b"".join(struct.pack(">hhh", f2(s), f2(p), f2(e)) for r in REGIONS
                                                        for s, p, e in r)
    blobs = [struct.pack(">HHH", 0, 0, len(ix)) + b"".join(struct.pack(">H", i) for i in ix) for ix in ivds]
    hdr = 8 + 4 * len(blobs)
    off, offs = hdr + len(rl), []
    for b in blobs:
        offs.append(off)
        off += len(b)
    store = struct.pack(">HIH", 1, hdr, len(blobs)) + b"".join(struct.pack(">I", o) for o in offs) + rl + b"".join(blobs)
    return struct.pack(">H", len(store)) + store


def f2(v):
    return int(round(v * F2))


def fdselect_bytes(fds, fmt):
    """FDSelect format 3 or 4 of the font DICT index per glyph"""
    ranges = []
    for g, f in enumerate(fds):
        if not ranges or ranges[-1][1] != f:
            ranges.append((g, f))
    if fmt == 4:
        return struct.pack(">BI", 4, len(ranges)) + b"".join(struct.pack(">IH", g, f) for g, f in ranges) + \
            struct.pack(">I", len(fds))
    return struct.pack(">BH", 3, len(ranges)) + b"".join(struct.pack(">HB", g, f) for g, f in ranges) + \
        struct.pack(">H", len(fds))


def cff2_pack(cs_items, gs_items, slots, fdselect, vstore, maxstack=None):
    """a CFF2 table: header, Top DICT, global subrs, VariationStore, CharStrings, FDSelect,
    FDArray, then per FDArray slot its Private DICT (slots: [(private(subrs offset) -> bytes,
    [local subr charstrings])]) and local subrs"""
    top_fixed = lambda cso, fda, fdso, vso: (dict_i32(cso) + b"\x11" + dict_i32(fda) + b"\x0c\x24" + dict_i32(fdso) +
                                            b"\x0c\x25" + dict_i32(vso) + b"\x18" +
                                            ((cff_int(maxstack) + b"\x19") if maxstack else b""))
    hdr_len = 5
    pos = hdr_len + len(top_fixed(0, 0, 0, 0))
    gs_idx = index2(gs_items)
    pos += len(gs_idx)
    vs_off = pos
    pos += len(vstore)
    cs_idx = index2(cs_items)
    cs_off = pos
    pos += len(cs_idx)
    fds_off = pos
    pos += len(fdselect)
    fd_dict_len = len(dict_i32(0) * 2) + 1
    fda_len = len(index2([bytes(fd_dict_len)] * len(slots)))
    fda_off = pos
    pos += fda_len
    privs, blobs = [], b""
    for private, subrs in slots:
        plen = len(private(0))
        priv = private(plen)
        privs.append((pos + len(blobs), len(priv)))
        blobs += priv + index2(subrs)
    fda = index2([dict_i32(sz) + dict_i32(off) + b"\x12" for off, sz in privs])
    assert len(fda) == fda_len
    top = top_fixed(cs_off, fda_off, fds_off, vs_off)
    return struct.pack(">BBBH", 2, 0, hdr_len, len(top)) + top + gs_idx + vstore + cs_idx + fdselect + fda + blobs


def cff2_table(variant):
    """variant 'otf': FDSelect format 3, explicit vsindex 0, two global subrs; 'ttc': FDSelect
    format 4, the font DICTs swapped, 1240 global subrs (bias 1131), maxstack 256"""
    ttc = variant == "ttc"
    fdmap = [1, 0] if ttc else [0, 1]                      # glyph's font DICT -> its FDArray index
    gsubrs = list(G_SUBRS) + ([[]] * (1240 - len(G_SUBRS)) if ttc else [])
    gb = subr_bias(len(gsubrs))
    gs_items = [cs2(s, 107, gb) for s in gsubrs]
    ls_items = [[cs2(s, subr_bias(len(L_SUBRS[fd])), gb) for s in L_SUBRS[fd]] for fd in range(2)]
    cs_items = [cs2(code, subr_bias(len(L_SUBRS[fd])), gb) for fd, code in G_GLYPHS]
    fdselect = fdselect_bytes([fdmap[fd] for fd, _ in G_GLYPHS], 4 if ttc else 3)
    slots = []
    for slot in range(2):
        fd = fdmap.index(slot)                              # the glyph font DICT in this slot
        explicit = (not ttc) or fd == 1                     # the ttc leaves vsindex 0 implicit
        slots.append((lambda rel, fd=fd, explicit=explicit: private2(fd, explicit, rel), ls_items[fd]))
    return cff2_pack(cs_items, gs_items, slots, fdselect, ivs_cff2(), 256 if ttc else None)


# limits.bin: one charstring per limit (font DICT, tokens, an outline at every instance:
# True / False = broken / None = no check)
LIMIT_GLYPHS = [
    (0, [*B((100, [10, 0]), (0, Z2)), "rmoveto", 500, "hlineto", 500, "vlineto", -500, "hlineto"], True),
    (0, [0, 0, "rmoveto"] + [1] * 301 + ["rlineto"], False),             # 301 operands > maxstack 300
    (0, [0, 0, "rmoveto", 10, 20, 5000, "blend", "hlineto"], False),       # blend: n x (k + 1) > the stack
    (0, [3, "vsindex", 0, 0, "rmoveto", 10, 20, 1, "blend", "hlineto"], False),   # no item variation data 3
    (0, [0, 0, "rmoveto", 10, 1, 1, 1, "blend", 1, "vsindex", 10, "hlineto"], False),  # vsindex after blend
    (0, [0, 0, "rmoveto", LS(0)], False),                                 # a subr calling itself: depth
    (0, [2, "vsindex", 0, 0, "rmoveto", 10, 1, "blend", "hlineto"], False),  # 600 regions > 512
    (0, [0, 0, "rmoveto"] + [1, 1] * 125 + ["rlineto"], True),           # 250 operands <= maxstack 300
    (0, [*([5] * 200), "hstemhm", "hintmask"], None),                     # mask bytes beyond the end
    (1, [0, 0, "rmoveto", LS(0), 10, "hlineto"], False),                  # font DICT 1: no Subrs (its
                                                                          # Private DICT's blend is short)
    (0, [0, 0, "rmoveto", 5, -5.5, "blend", 10, "hlineto"], False),     # a negative count
    (0, [0, 0, "rmoveto", 32767.5, "vsindex", 10, "hlineto"], None),       # a huge vsindex, no blend
]
LIMIT_IVDS = [[0, 4], [1], [0] * 600]


def limits_font():
    """CFF2 limits (fonttest --selftest): maxstack 300, blends with too few operands, a missing
    item variation data, vsindex after blend, subr recursion, 600 regions, mask bytes past the
    end, a Private DICT whose blend lacks operands (font DICT 1 loses its Subrs)"""
    lsubr0 = cs2([LS(0)])
    good = lambda rel: cff_int(0) + b"\x16" + dict_i32(rel) + b"\x13"
    short = lambda rel: cff_int(7) + cff_int(2) + b"\x17" + b"\x06" + dict_i32(rel) + b"\x13"
    cs_items = [cs2(code) for _, code, _ in LIMIT_GLYPHS]
    fdselect = fdselect_bytes([fd for fd, _, _ in LIMIT_GLYPHS], 3)
    cff2 = cff2_pack(cs_items, [], [(good, [lsubr0]), (short, [lsubr0])], fdselect, ivs_cff2(LIMIT_IVDS), 300)
    box = [[(100, 0, 1), (600, 0, 1), (600, 500, 1), (100, 500, 1)]]
    glyphs = [(box if k == 0 else [], 600) for k in range(len(LIMIT_GLYPHS))]
    t = common_tables(glyphs, [(1, 0x409, "jfgui CFF2 Limits"), (2, 0x409, "Regular")], 400, cmap12({0x41: 0}))
    t["CFF2"] = cff2
    t["fvar"] = fvar_table(AXES, [])
    return sfnt(t, 0x4F54544F)


# ------------------------------------------------------------------ the model
def region_factor(v, start, peak, end):
    """one region axis at the F2Dot14 coordinate v (HarfBuzz's VarRegionAxis::evaluate)"""
    if peak == 0 or v == peak:
        return 1.0
    if v == 0:
        return 0.0
    if start > peak or peak > end or (start < 0 < end):
        return 1.0
    if v <= start or end <= v:
        return 0.0
    return (v - start) / (peak - start) if v < peak else (end - v) / (end - peak)


def scalars(coords):
    """the scalars of every item variation data's regions at coords"""
    out = []
    for ix in IVDS:
        row = []
        for ri in ix:
            s = 1.0
            for a, (st, pk, en) in enumerate(REGIONS[ri]):
                s *= region_factor(coords[a], f2(st), f2(pk), f2(en))
            row.append(s)
        out.append(row)
    return out


def cff2_eval(code, fd, sc):
    """the outline of a CFF2 charstring at the scalars sc -> contours [(x, y, tag)], tag 1
    on-curve / 2 cubic control (reference evaluation from the specification)"""
    contours = []
    st = []
    cur = None
    state = dict(x=0.0, y=0.0, stems=0, vsi=fd if fd == 1 else 0)

    def moveto(nx, ny):
        nonlocal cur
        state["x"], state["y"] = nx, ny
        cur = [(nx, ny, 1)]
        contours.append(cur)

    def line(dx, dy):
        state["x"] += dx
        state["y"] += dy
        cur.append((state["x"], state["y"], 1))

    def curve(a, b, c, d, e, f):
        x, y = state["x"], state["y"]
        c1 = (x + a, y + b)
        c2 = (c1[0] + c, c1[1] + d)
        p = (c2[0] + e, c2[1] + f)
        cur.extend([(c1[0], c1[1], 2), (c2[0], c2[1], 2), (p[0], p[1], 1)])
        state["x"], state["y"] = p

    def run(code):
        i = 0
        while i < len(code):
            t = code[i]
            i += 1
            if isinstance(t, tuple):
                kind, k = t
                run((L_SUBRS[fd] if kind == "lsubr" else G_SUBRS)[k])
                continue
            if isinstance(t, (int, float)):
                st.append(float(t))
                continue
            if t.startswith("mask:"):
                continue
            if t == "blend":
                n = int(st.pop())
                s = sc[state["vsi"]]
                k = len(s)
                base = len(st) - n * (k + 1)
                assert base >= 0
                res = [st[base + j] + sum(st[base + n + j * k + r] * s[r] for r in range(k)) for j in range(n)]
                del st[base:]
                st.extend(res)
                continue
            if t == "vsindex":
                state["vsi"] = int(st.pop())
                st.clear()
            elif t in ("hstem", "vstem", "hstemhm", "vstemhm"):
                state["stems"] += len(st) // 2
                st.clear()
            elif t in ("hintmask", "cntrmask"):
                state["stems"] += len(st) // 2
                st.clear()
            elif t == "rmoveto":
                moveto(state["x"] + st[0], state["y"] + st[1])
                st.clear()
            elif t == "rlineto":
                for k in range(0, len(st) - 1, 2):
                    line(st[k], st[k + 1])
                st.clear()
            elif t in ("hlineto", "vlineto"):
                h = t == "hlineto"
                for v in st:
                    line(v, 0) if h else line(0, v)
                    h = not h
                st.clear()
            elif t == "rrcurveto":
                for k in range(0, len(st) - 5, 6):
                    curve(*st[k:k + 6])
                st.clear()
            elif t == "rcurveline":
                k = 0
                while len(st) - k >= 8:
                    curve(*st[k:k + 6])
                    k += 6
                line(st[k], st[k + 1])
                st.clear()
            elif t == "rlinecurve":
                k = 0
                while len(st) - k > 6:
                    line(st[k], st[k + 1])
                    k += 2
                curve(*st[k:k + 6])
                st.clear()
            elif t == "hhcurveto":
                k, dy1 = 0, 0.0
                if len(st) % 2:
                    dy1, k = st[0], 1
                while k + 4 <= len(st):
                    curve(st[k], dy1, st[k + 1], st[k + 2], st[k + 3], 0)
                    dy1 = 0.0
                    k += 4
                st.clear()
            elif t == "vvcurveto":
                k, dx1 = 0, 0.0
                if len(st) % 2:
                    dx1, k = st[0], 1
                while k + 4 <= len(st):
                    curve(dx1, st[k], st[k + 1], st[k + 2], 0, st[k + 3])
                    dx1 = 0.0
                    k += 4
                st.clear()
            elif t in ("vhcurveto", "hvcurveto"):
                h = t == "hvcurveto"
                k = 0
                while k + 4 <= len(st):
                    last = len(st) - k == 5
                    e = st[k + 4] if last else 0
                    if h:
                        curve(st[k], 0, st[k + 1], st[k + 2], e, st[k + 3])
                    else:
                        curve(0, st[k], st[k + 1], st[k + 2], st[k + 3], e)
                    k += 5 if last else 4
                    h = not h
                st.clear()
            elif t == "flex":
                curve(*st[0:6])
                curve(*st[6:12])
                st.clear()
            elif t == "hflex":
                y0 = state["y"]
                a = st
                curve(a[0], 0, a[1], a[2], a[3], 0)
                curve(a[4], 0, a[5], y0 - state["y"], a[6], 0)
                st.clear()
            elif t == "hflex1":
                y0 = state["y"]
                a = st
                curve(a[0], a[1], a[2], a[3], a[4], 0)
                curve(a[5], 0, a[6], a[7], a[8], y0 - (state["y"] + a[7]))
                st.clear()
            elif t == "flex1":
                x0, y0 = state["x"], state["y"]
                a = st
                dx = sum(a[0:10:2])
                dy = sum(a[1:10:2])
                curve(*a[0:6])
                xc = state["x"] + a[6] + a[8]
                yc = state["y"] + a[7] + a[9]
                if abs(dx) > abs(dy):
                    curve(a[6], a[7], a[8], a[9], a[10], y0 - yc)
                else:
                    curve(a[6], a[7], a[8], a[9], x0 - xc, a[10])
                st.clear()
            else:
                raise ValueError(t)
    run(code)
    return contours


def hv_delta(gid, coords):
    s = 0.0
    for ri, d in enumerate(HV_ROWS[gid]):
        sc = 1.0
        for a, (st, pk, en) in enumerate(REGIONS[ri]):
            sc *= region_factor(coords[a], f2(st), f2(pk), f2(en))
        s += sc * d
    return s


def instance(user):
    coords = normalize(AXES, None, user)
    sc = scalars(coords)
    glyphs = []
    for gid, (fd, code) in enumerate(G_GLYPHS):
        cont = cff2_eval(code, fd, sc)
        adv = ADV[gid] + math.floor(hv_delta(gid, coords) + .5)
        glyphs.append((cont, adv))
    return coords, glyphs


# ------------------------------------------------------------------ fonts
def num(v):
    return int(v) if v == int(v) else float(v)


def cff1_charstring(contours):
    """a CFF charstring drawing exactly these points (rmoveto, rlineto, rrcurveto, endchar)"""
    t, x, y = [], 0.0, 0.0
    for c in contours:
        t += [num(c[0][0] - x), num(c[0][1] - y), "rmoveto"]
        x, y = c[0][0], c[0][1]
        i = 1
        while i < len(c):
            if c[i][2] == 2:
                a, b, e = c[i], c[i + 1], c[i + 2]
                t += [num(a[0] - x), num(a[1] - y), num(b[0] - a[0]), num(b[1] - a[1]), num(e[0] - b[0]),
                      num(e[1] - b[1]), "rrcurveto"]
                x, y = e[0], e[1]
                i += 3
            else:
                t += [num(c[i][0] - x), num(c[i][1] - y), "rlineto"]
                x, y = c[i][0], c[i][1]
                i += 1
    return cs2(t + ["endchar"])


def cff1_table(psname, charstrings):
    hdr = bytes([1, 0, 4, 4])
    name_idx = cff_index([psname])
    string_idx = cff_index([])
    gsubr_idx = cff_index([])
    top_fixed = lambda cso, psz, po: dict_i32(cso) + b"\x11" + dict_i32(psz) + dict_i32(po) + b"\x12"
    top_len = len(top_fixed(0, 0, 0))
    cs_off = len(hdr) + len(name_idx) + len(cff_index([bytes(top_len)])) + len(string_idx) + len(gsubr_idx)
    cs_idx = cff_index(charstrings)
    priv_off = cs_off + len(cs_idx)
    private = dict_i32(0) + b"\x14" + dict_i32(0) + b"\x15"
    top = top_fixed(cs_off, len(private), priv_off)
    return hdr + name_idx + cff_index([top]) + string_idx + gsubr_idx + cs_idx + private


def bbox_of(glyphs):
    pts = [p for cont, _ in glyphs for c in cont for p in c]
    return (math.floor(min(p[0] for p in pts)), math.floor(min(p[1] for p in pts)),
            math.ceil(max(p[0] for p in pts)), math.ceil(max(p[1] for p in pts)))


def common_tables(glyphs, names, weight, cmap):
    n = len(glyphs)
    def lsb(cont):
        return math.floor(min(p[0] for c in cont for p in c)) if cont else 0
    return {
        "head": head_table(1000, bbox_of(glyphs), 0, 0),
        "hhea": hhea_table(850, -250, 50, max(a for _, a in glyphs), n),
        "maxp": struct.pack(">IH", 0x5000, n),
        "hmtx": b"".join(struct.pack(">Hh", a, lsb(c)) for c, a in glyphs),
        "cmap": cmap,
        "OS/2": os2_table(weight, 0x40, 500, 700, 300, 50, 800, -200, 100),
        "post": post_table(-100, 50),
        "name": name_table(names),
    }


def cff2_tables(variant, names, cmap):
    _, glyphs = instance({})
    t = common_tables([(c, ADV[g]) for g, (c, _) in enumerate(glyphs)], names, 400, cmap)
    t["CFF2"] = cff2_table(variant)
    t["fvar"] = fvar_table(AXES, INSTANCES)
    store = ivs(2, REGIONS, [(HV_ROWS, 1, False)])
    t["HVAR"] = hvar_table(store, [(0, g) for g in range(len(G_GLYPHS))])
    return t


def otc(faces):
    """an OpenType collection of faces {tag: bytes}; equal tables are stored once"""
    n = len(faces)
    hdr_len = 12 + 4 * n
    dir_lens = [12 + 16 * len(f) for f in faces]
    pos = hdr_len + sum(dir_lens)
    where, body = {}, b""
    for f in faces:
        for tag in sorted(f):
            d = f[tag]
            if d not in where:
                where[d] = pos + len(body)
                body += d + bytes((-len(d)) % 4)
    out = b"ttcf" + struct.pack(">II", 0x10000, n)
    off = hdr_len
    for dl in dir_lens:
        out += struct.pack(">I", off)
        off += dl
    for f in faces:
        tags = sorted(f)
        nt = len(tags)
        es = int(math.log2(nt))
        sr = (1 << es) * 16
        out += struct.pack(">IHHHH", 0x4F54544F, nt, sr, es, nt * 16 - sr)
        for t in tags:
            out += t.encode("latin-1") + struct.pack(">III", checksum(f[t]), where[f[t]], len(f[t]))
    assert len(out) == pos
    return out + body


def build_ref(key, user, label, weight):
    _, glyphs = instance(user)
    names = [(1, 0x409, f"jfgui CFF2 Ref {label}"), (2, 0x409, "Regular"), (4, 0x409, f"jfgui CFF2 Ref {label} Regular")]
    t = common_tables(glyphs, names, weight, cmap12(CMAP))
    t["CFF "] = cff1_table(f"jfguiCFF2Ref{label}".encode(), [cff1_charstring(c) for c, _ in glyphs])
    return sfnt(t, 0x4F54544F)


# ------------------------------------------------------------------ main
def check_exact(glyphs):
    """the comparison instances: every coordinate a multiple of 1/2 (exact in f32 and 16.16)"""
    for cont, _ in glyphs:
        for c in cont:
            for x, y, _ in c:
                assert x * 2 == int(x * 2) and y * 2 == int(y * 2) and abs(x) < 1024 and abs(y) < 1024, (x, y)


def check_dyadic(glyphs, coords):
    """the other instances: every coordinate and advance delta a multiple of 2^-14 below 1024
    (24 bits: f32 computes them exactly, like double here)"""
    for gid, (cont, adv) in enumerate(glyphs):
        vals = [v for c in cont for x, y, _ in c for v in (x, y)] + [hv_delta(gid, coords)]
        for v in vals:
            assert v * F2 == int(v * F2) and abs(v) < 1024, (gid, v)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    REF.mkdir(parents=True, exist_ok=True)
    otf = sfnt(cff2_tables("otf", NAMES, cmap12(CMAP)), 0x4F54544F)
    (OUT / "jfcff2.otf").write_bytes(otf)
    faces = []
    for tag, gid in (("JP", 6), ("SC", 7)):
        cm = dict(CMAP)
        cm[0x4E2D] = gid
        names = [(1, 0x409, f"jfgui CFF2 {tag}"), (2, 0x409, "Regular"), (4, 0x409, f"jfgui CFF2 {tag} Regular"),
                 (256, 0x409, "Weight"), (256, 0x407, "Strichstärke"), (257, 0x409, "Width"), (257, 0x407, "Breite"),
                 (260, 0x409, "Light"), (261, 0x409, "Regular"), (262, 0x409, "Semibold"), (263, 0x409, "Condensed Black")]
        faces.append(cff2_tables("ttc", names, cmap4(cm)))
    (OUT / "jfcff2.ttc").write_bytes(otc(faces))
    (OUT / "limits.bin").write_bytes(limits_font())
    for key, user, label, weight in REFS:
        coords, glyphs = instance(user)
        check_exact(glyphs)
        assert all(hv_delta(g, coords) == int(hv_delta(g, coords)) for g in range(len(G_GLYPHS)))
        (REF / f"ref-{key}.otf").write_bytes(build_ref(key, user, label, weight))
    # expected.txt: every glyph at each instance (the odd one too)
    exp = ""
    cases = [("jfcff2.otf", 0, u) for _, u, _, _ in REFS] + [("jfcff2.otf", 0, u) for u in ODDS]
    cases.append(("jfcff2.ttc", 1, {"wght": 650}))
    for path, face, user in cases:
        coords, glyphs = instance(user)
        check_dyadic(glyphs, coords)
        spec = ",".join(f"{k}={v:g}" for k, v in user.items()) or "-"
        exp += f"var {path} {face} {spec} coords {' '.join(str(c) for c in coords)}\n"
        for gid, (cont, adv) in enumerate(glyphs):
            exp += fmt_outline(f"g{gid}", gid, adv, [c for c in cont if c])
    (OUT / "expected.txt").write_text(exp, encoding="utf-8", newline="\n")
    print("wrote", OUT, "and", REF)


if __name__ == "__main__":
    main()
