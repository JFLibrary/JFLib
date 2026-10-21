#!/usr/bin/env python3
"""font_fixtures.py -- synthetic test fonts for `fonttest --selftest` (apps/fonttest/tests).

Writes apps/fonttest/tests/fixtures/:
  mini.ttf   TrueType: simple glyphs (all flag / coordinate encodings), composites (offset,
             scale, nested), an lsb != xMin glyph, cmap format 4 (3,1), kern table, COLR/CPAL
             layers, German + English name records
  mini.otf   CFF: rmoveto with width, lines, vh/hv/rr curves, hints + hintmask, local and
             global subrs, flex / hflex / hflex1 / flex1; cmap format 12 (3,10); GPOS pair
             adjustment format 1 and format 2 (the latter through an extension lookup)
  mini.ttc   collection: mini.ttf as face 0, a bold variant as face 1
  mac.ttf    TrueType with only a Mac Roman (1,0) format 0 cmap and an Apple kern v1 table
  cid.otf    CID-keyed CFF: FDArray with two font DICTs (own local subrs), FDSelect format 3
  cbdt.ttf   CBDT/CBLC colour bitmaps (from font_check.make_cbdt_font)
  expected.txt  the output fonttest --selftest must produce, computed from the glyph
             definitions here (independent of the assembly parser)

Only synthetic geometry: the files may be redistributed freely.
  python tools/font_fixtures.py
"""
import math
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIX = ROOT / "apps" / "fonttest" / "tests" / "fixtures"
sys.path.insert(0, str(Path(__file__).resolve().parent))


# ------------------------------------------------------------------ sfnt assembly
def checksum(b):
    b = b + bytes((-len(b)) % 4)
    return sum(struct.unpack(f">{len(b) // 4}I", b)) & 0xFFFFFFFF


def sfnt(tables, version=0x10000):
    tags = sorted(tables)
    n = len(tags)
    es = int(math.log2(n))
    sr = (1 << es) * 16
    head = struct.pack(">IHHHH", version, n, sr, es, n * 16 - sr)
    off = 12 + 16 * n
    dirs, body = b"", b""
    for t in tags:
        d = tables[t]
        dirs += t.encode("latin-1") + struct.pack(">III", checksum(d), off + len(body), len(d))
        body += d + bytes((-len(d)) % 4)
    return head + dirs + body


def ttc(fonts):
    """collection of complete sfnt files (tables are not shared)"""
    n = len(fonts)
    hdr = 12 + 4 * n
    offs, blob = [], b""
    for f in fonts:
        base = hdr + len(blob)
        offs.append(base)
        num = struct.unpack_from(">H", f, 4)[0]
        g = bytearray(f)
        for i in range(num):
            o = 12 + 16 * i
            struct.pack_into(">I", g, o + 8, struct.unpack_from(">I", f, o + 8)[0] + base)
        blob += bytes(g) + bytes((-len(g)) % 4)
    return b"ttcf" + struct.pack(">II", 0x10000, n) + b"".join(struct.pack(">I", o) for o in offs) + blob


def name_table(recs):
    """recs: [(nameID, lang, text)] -> Windows Unicode records (sorted)"""
    recs = sorted(recs, key=lambda r: (3, 1, r[1], r[0]))
    strs, rr = b"", b""
    for nid, lang, text in recs:
        v = text.encode("utf-16-be")
        rr += struct.pack(">HHHHHH", 3, 1, lang, nid, len(v), len(strs))
        strs += v
    return struct.pack(">HHH", 0, len(recs), 6 + 12 * len(recs)) + rr + strs


def os2_table(weight, fs_sel, xh, caph, st_pos, st_size, asc, desc, gap):
    t = struct.pack(">HhHHH", 4, 500, weight, 5, 0)              # version .. fsType
    t += struct.pack(">hhhhhhhhhh", 650, 700, 0, 140, 650, 700, 0, 480, st_size, st_pos)
    t += struct.pack(">h", 0) + bytes(10) + bytes(16) + b"ASMG"   # family class, panose, ranges, vendor
    t += struct.pack(">HHH", fs_sel, 0x20, 0xFFFF)                 # fsSelection, first, last char
    t += struct.pack(">hhhHH", asc, desc, gap, asc, -desc)         # typo + win
    t += struct.pack(">II", 1, 0)                                  # code page ranges
    t += struct.pack(">hhHHH", xh, caph, 0, 0x20, 1)               # v2+: xHeight, capHeight ...
    assert len(t) == 96
    return t


def head_table(upem, bbox, macstyle, locformat):
    return struct.pack(">IIIIHHqqhhhhHHhhh", 0x10000, 0x10000, 0, 0x5F0F3CF5, 3, upem, 0, 0,
                       *bbox, macstyle, 8, 2, locformat, 0)


def hhea_table(asc, desc, gap, advmax, nhm):
    return struct.pack(">IhhhHhhhhhhhhhhhH", 0x10000, asc, desc, gap, advmax, 0, 0, advmax, 1, 0, 0,
                       0, 0, 0, 0, 0, nhm)


def post_table(ul_pos, ul_thick):
    return struct.pack(">IihhI", 0x30000, 0, ul_pos, ul_thick, 0) + bytes(16)


def cmap4(mapping):
    """(3,1) format 4 from {cp: gid} (BMP), one segment per run, plus the 0xFFFF segment"""
    cps = sorted(c for c in mapping if c <= 0xFFFF)
    segs = []
    for c in cps:
        if segs and c == segs[-1][1] + 1 and mapping[c] == mapping[segs[-1][1]] + 1:
            segs[-1][1] = c
        else:
            segs.append([c, c])
    segs.append([0xFFFF, 0xFFFF])
    n = len(segs)
    ends = b"".join(struct.pack(">H", e) for s, e in segs)
    starts = b"".join(struct.pack(">H", s) for s, e in segs)
    deltas = b"".join(struct.pack(">H", ((mapping[s] - s) & 0xFFFF) if s != 0xFFFF else 1) for s, e in segs)
    ros = bytes(2 * n)
    es = int(math.log2(n))
    body = (struct.pack(">HHHH", 2 * n, 2 * (1 << es), es, 2 * n - 2 * (1 << es)) + ends + b"\0\0" + starts +
            deltas + ros)
    sub = struct.pack(">HHH", 4, 6 + len(body), 0) + body
    return struct.pack(">HHHHI", 0, 1, 3, 1, 12) + sub


def cmap12(mapping):
    groups = []
    for c in sorted(mapping):
        g = mapping[c]
        if groups and c == groups[-1][1] + 1 and g == groups[-1][2] + (c - groups[-1][0]):
            groups[-1][1] = c
        else:
            groups.append([c, c, g])
    sub = struct.pack(">HHIII", 12, 0, 16 + 12 * len(groups), 0, len(groups))
    sub += b"".join(struct.pack(">III", *g) for g in groups)
    return struct.pack(">HHHHI", 0, 1, 3, 10, 12) + sub


# ------------------------------------------------------------------ glyf
def encode_simple(contours):
    """TrueType simple glyph with the full flag compression (short / same / repeat)"""
    pts = [p for c in contours for p in c]
    ends, n = [], 0
    for c in contours:
        n += len(c)
        ends.append(n - 1)
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    bbox = (min(xs), min(ys), max(xs), max(ys))
    flags, xb, yb = [], b"", b""
    px = py = 0
    for x, y, on in pts:
        f = 1 if on else 0
        dx, dy = x - px, y - py
        px, py = x, y
        if dx == 0:
            f |= 0x10
        elif -255 <= dx <= 255:
            f |= 0x02 | (0x10 if dx > 0 else 0)
            xb += bytes([abs(dx)])
        else:
            xb += struct.pack(">h", dx)
        if dy == 0:
            f |= 0x20
        elif -255 <= dy <= 255:
            f |= 0x04 | (0x20 if dy > 0 else 0)
            yb += bytes([abs(dy)])
        else:
            yb += struct.pack(">h", dy)
        flags.append(f)
    fb, i = b"", 0
    while i < len(flags):
        j = i
        while j + 1 < len(flags) and flags[j + 1] == flags[i] and j - i < 254:
            j += 1
        if j > i:
            fb += bytes([flags[i] | 0x08, j - i])
        else:
            fb += bytes([flags[i]])
        i = j + 1
    data = struct.pack(">hhhhh", len(contours), *bbox) + b"".join(struct.pack(">H", e) for e in ends)
    data += struct.pack(">H", 0) + fb + xb + yb
    return data, bbox


def encode_composite(comps, bbox):
    """comps: [(gid, dx, dy, scale or None)]"""
    data = struct.pack(">hhhhh", -1, *bbox)
    for i, (gid, dx, dy, scale) in enumerate(comps):
        fl = 0x02
        if not (-128 <= dx <= 127 and -128 <= dy <= 127):
            fl |= 0x01
        if scale is not None:
            fl |= 0x08
        if i + 1 < len(comps):
            fl |= 0x20
        data += struct.pack(">HH", fl, gid)
        data += struct.pack(">hh", dx, dy) if fl & 1 else struct.pack(">bb", dx, dy)
        if scale is not None:
            data += struct.pack(">h", int(round(scale * 16384)))
    return data


# TrueType glyph set (upem 1000); contours clockwise
TT_GLYPHS = [
    # 0 .notdef: box with a hole
    dict(adv=600, contours=[[(50, 0, 1), (50, 700, 1), (550, 700, 1), (550, 0, 1)],
                            [(100, 50, 1), (500, 50, 1), (500, 650, 1), (100, 650, 1)]]),
    # 1 A: triangle, big deltas (16-bit coordinates)
    dict(adv=1000, contours=[[(100, 0, 1), (500, 700, 1), (900, 0, 1)]]),
    # 2 V: lsb 10 units left of xMin (rasterizers shift the outline by lsb - xMin)
    dict(adv=1000, lsb=90, contours=[[(100, 700, 1), (900, 700, 1), (500, 0, 1)]]),
    # 3 O: quadratic circle, off-curve start, consecutive off-curve points (implied midpoints)
    dict(adv=900, contours=[[(450, 700, 0), (800, 700, 0), (800, 0, 0), (100, 0, 0), (100, 700, 0)]]),
    # 4 H: two contours, repeated flags
    dict(adv=900, contours=[[(100, 0, 1), (100, 700, 1), (200, 700, 1), (200, 0, 1)],
                            [(700, 0, 1), (700, 700, 1), (800, 700, 1), (800, 0, 1)]]),
    # 5 Adieresis: composite A + dieresis at (0, 800) (words), then 6 at (-300, 0) (bytes? no)
    dict(adv=1000, comps=[(1, 0, 0, None), (6, 0, 800, None)]),
    # 6 dieresis: two squares
    dict(adv=0, contours=[[(300, 0, 1), (300, 100, 1), (400, 100, 1), (400, 0, 1)],
                          [(600, 0, 1), (600, 100, 1), (700, 100, 1), (700, 0, 1)]]),
    # 7 S: scaled composite (O at half size, offset 225,100), nested composite 5 at (0,0) scale 0.5
    dict(adv=900, comps=[(3, 225, 100, 0.5), (5, 10, -20, 0.5)]),
    # 8 C: COLR base glyph (its own outline: a square)
    dict(adv=800, contours=[[(100, 0, 1), (100, 600, 1), (700, 600, 1), (700, 0, 1)]]),
    # 9, 10: layers
    dict(adv=800, contours=[[(100, 0, 1), (100, 600, 1), (700, 600, 1), (700, 0, 1)]]),
    dict(adv=800, contours=[[(300, 200, 1), (300, 400, 1), (500, 400, 1), (500, 200, 1)]]),
    # 11 space: empty
    dict(adv=250, contours=[]),
]
TT_CMAP = {0x41: 1, 0x56: 2, 0x4F: 3, 0x48: 4, 0xC4: 5, 0x53: 7, 0x43: 8, 0x20: 11}
TT_KERN = {(1, 2): -80, (2, 1): -60, (3, 1): 12}


def resolve(glyphs, gid, depth=0):
    """fully resolved points [(x, y, on)] per contour, as the loader produces them"""
    g = glyphs[gid]
    if "comps" not in g:
        return [[(float(x), float(y), on) for x, y, on in c] for c in g["contours"]]
    out = []
    for cg, dx, dy, scale in g["comps"]:
        for c in resolve(glyphs, cg, depth + 1):
            if scale is not None:
                s = round(scale * 16384) / 16384.0
                c = [(x * s, y * s, on) for x, y, on in c]
            out.append([(x + dx, y + dy, on) for x, y, on in c])
    return out


def build_ttf(glyphs, names, weight=400, fs_sel=0x40, macstyle=0, colr=True, kern=TT_KERN):
    glyf, loca, bboxes = b"", [], []
    for gid, g in enumerate(glyphs):
        loca.append(len(glyf))
        if "comps" in g:
            pts = [p for c in resolve(glyphs, gid) for p in c]
            bbox = (int(math.floor(min(p[0] for p in pts))), int(math.floor(min(p[1] for p in pts))),
                    int(math.ceil(max(p[0] for p in pts))), int(math.ceil(max(p[1] for p in pts))))
            d = encode_composite(g["comps"], bbox)
        elif g["contours"]:
            d, bbox = encode_simple(g["contours"])
        else:
            d, bbox = b"", (0, 0, 0, 0)
        g["bbox"] = bbox
        bboxes.append(bbox)
        glyf += d + bytes((-len(d)) % 4)
    loca.append(len(glyf))
    n = len(glyphs)
    allb = [b for b, g in zip(bboxes, glyphs) if g.get("contours") or g.get("comps")]
    fb = (min(b[0] for b in allb), min(b[1] for b in allb), max(b[2] for b in allb), max(b[3] for b in allb))
    hmtx = b"".join(struct.pack(">Hh", g["adv"], g.get("lsb", g["bbox"][0])) for g in glyphs)
    tables = {
        "head": head_table(1000, fb, macstyle, 1),
        "hhea": hhea_table(850, -250, 50, max(g["adv"] for g in glyphs), n),
        "maxp": struct.pack(">IHHHHHHHHHHHHHH", 0x10000, n, 16, 4, 16, 4, 2, 0, 0, 0, 0, 0, 0, 2, 2),
        "hmtx": hmtx,
        "loca": b"".join(struct.pack(">I", o) for o in loca),
        "glyf": glyf,
        "cmap": cmap4(TT_CMAP),
        "OS/2": os2_table(weight, fs_sel, 500, 700, 300, 50, 800, -200, 100),
        "post": post_table(-100, 50),
        "name": name_table(names),
    }
    if kern:
        pairs = sorted(kern.items())
        es = int(math.log2(len(pairs)))
        sub = struct.pack(">HHHH", len(pairs), 6 * (1 << es), es, 6 * len(pairs) - 6 * (1 << es))
        sub += b"".join(struct.pack(">HHh", l, r, v) for (l, r), v in pairs)
        tables["kern"] = struct.pack(">HH", 0, 1) + struct.pack(">HHH", 0, 6 + len(sub), 0x0001) + sub
    if colr:
        base = struct.pack(">HHH", 8, 0, 2)
        layers = struct.pack(">HH", 9, 0) + struct.pack(">HH", 10, 0xFFFF)
        tables["COLR"] = struct.pack(">HHIIH", 0, 1, 14, 14 + len(base), 2) + base + layers
        tables["CPAL"] = struct.pack(">HHHHIH", 0, 1, 1, 1, 14, 0) + bytes([0x20, 0x40, 0xE0, 0xFF])
    return sfnt(tables)


# ------------------------------------------------------------------ CFF
def cff_int(v):
    if -107 <= v <= 107:
        return bytes([v + 139])
    if 108 <= v <= 1131:
        v -= 108
        return bytes([(v >> 8) + 247, v & 255])
    if -1131 <= v <= -108:
        v = -v - 108
        return bytes([(v >> 8) + 251, v & 255])
    return b"\x1c" + struct.pack(">h", v)


def cs(*items):
    """Type 2 charstring from ints (operands) and strings (operators, 'hintmask:XX' raw bytes)"""
    ops = {"hstem": b"\x01", "vstem": b"\x03", "vmoveto": b"\x04", "rlineto": b"\x05", "hlineto": b"\x06",
           "vlineto": b"\x07", "rrcurveto": b"\x08", "callsubr": b"\x0a", "return": b"\x0b",
           "endchar": b"\x0e", "hstemhm": b"\x12", "hintmask": b"\x13", "cntrmask": b"\x14",
           "rmoveto": b"\x15", "hmoveto": b"\x16", "vstemhm": b"\x17", "rcurveline": b"\x18",
           "rlinecurve": b"\x19", "vvcurveto": b"\x1a", "hhcurveto": b"\x1b", "callgsubr": b"\x1d",
           "vhcurveto": b"\x1e", "hvcurveto": b"\x1f", "hflex": b"\x0c\x22", "flex": b"\x0c\x23",
           "hflex1": b"\x0c\x24", "flex1": b"\x0c\x25", "fixed": None}
    out = b""
    for it in items:
        if isinstance(it, int):
            out += cff_int(it)
        elif isinstance(it, float):
            out += b"\xff" + struct.pack(">i", int(round(it * 65536)))
        elif it.startswith("mask:"):
            out += bytes.fromhex(it[5:])
        else:
            out += ops[it]
    return out


def cff_index(items):
    if not items:
        return struct.pack(">H", 0)
    offs = [1]
    for it in items:
        offs.append(offs[-1] + len(it))
    osz = 1 if offs[-1] < 256 else (2 if offs[-1] < 65536 else 4)
    fmt = {1: ">B", 2: ">H", 4: ">I"}[osz]
    return (struct.pack(">HB", len(items), osz) + b"".join(struct.pack(fmt, o) for o in offs) +
            b"".join(items))


def dict_i32(v):
    return b"\x1d" + struct.pack(">i", v)


GSUBR_BIAS, LSUBR_BIAS = 107, 107
# local subr 0: up 700; global subr 0: right 300
CFF_LSUBRS = [cs(0, 700, "rlineto", "return")]
CFF_GSUBRS = [cs(300, 0, "rlineto", "return")]
CFF_GLYPHS = [
    # 0 .notdef
    dict(adv=500, cs=cs(50, 0, "rmoveto", 400, "hlineto", 700, "vlineto", -400, "hlineto", "endchar")),
    # 1 A: width + rmoveto, rlineto
    dict(adv=1000, cs=cs(600, 100, 0, "rmoveto", 400, 700, 400, -700, "rlineto", "endchar")),
    # 2 V
    dict(adv=1000, cs=cs(100, 700, "rmoveto", 800, "hlineto", -400, -700, "rlineto", "endchar")),
    # 3 O: two vhcurveto calls (vertical / horizontal alternation, 8 arguments each)
    dict(adv=900, cs=cs(100, 350, "rmoveto", 193, 157, 157, 193, 193, 157, -157, -193, "vhcurveto",
                        -193, -157, -157, -193, -193, -157, 157, 193, "vhcurveto", "endchar")),
    # 4 B: stems, hintmask (3 stems -> 1 byte), local + global subrs
    dict(adv=900, cs=cs(10, 20, 30, 40, "hstemhm", 50, 60, "vstemhm", "hintmask", "mask:E0", 100, 0,
                        "rmoveto", -LSUBR_BIAS, "callsubr", -GSUBR_BIAS, "callgsubr", 0, -700, "rlineto",
                        "endchar")),
    # 5 F: flex (two curves, 12 deltas + depth)
    dict(adv=900, cs=cs(100, 100, "rmoveto", 50, 50, 50, 50, 50, 0, 50, 0, 50, -50, 50, -50, 50, "flex",
                        0, -100, "rlineto", "endchar")),
    # 6 G: hflex
    dict(adv=900, cs=cs(100, 100, "rmoveto", 50, 50, 20, 50, 50, 50, 50, "hflex", 0, -100, "rlineto",
                        "endchar")),
    # 7 J: hflex1 then flex1
    dict(adv=900, cs=cs(100, 100, "rmoveto", 50, 10, 50, 20, 50, 50, 50, -20, 50, "hflex1",
                        10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, "flex1", "endchar")),
    # 8 Q: hh / vv / hv curves, rcurveline, rlinecurve, fixed-point operand
    dict(adv=900, cs=cs(100, 100, "rmoveto", 20, 50, 50, 50, 50, "hhcurveto", 30, 50, 50, 50, 50,
                        "vvcurveto", 50, 50, 50, 50, 20, "hvcurveto", 10, 10, 10, 10, 10, 10, 30, 0,
                        "rcurveline", 0, -10, 0, -10, 10, 10, 10, 10, 10, 10.5, "rlinecurve", "endchar")),
]
OTF_CMAP = {0x41: 1, 0x56: 2, 0x4F: 3, 0x42: 4, 0x46: 5, 0x47: 6, 0x4A: 7, 0x51: 8, 0x1F600: 3}


def t2_eval(code, lsubrs, gsubrs):
    """reference Type 2 path evaluation for the operators used above -> contours of
    (x, y, tag) with tag 1 on-curve / 2 cubic control"""
    st, contours, cur = [], [], None
    x = y = 0.0
    state = dict(stems=0, width_done=False)

    def pop_width(expect_even=None, n=None):
        if not state["width_done"]:
            state["width_done"] = True
            if n is not None and len(st) > n:
                st.pop(0)
            elif expect_even and len(st) % 2:
                st.pop(0)

    def moveto(nx, ny):
        nonlocal x, y, cur
        x, y = nx, ny
        cur = [(x, y, 1)]
        contours.append(cur)

    def line(dx, dy):
        nonlocal x, y
        x += dx
        y += dy
        cur.append((x, y, 1))

    def curve(a, b, c, d, e, f):
        nonlocal x, y
        c1 = (x + a, y + b)
        c2 = (c1[0] + c, c1[1] + d)
        p = (c2[0] + e, c2[1] + f)
        cur.extend([(c1[0], c1[1], 2), (c2[0], c2[1], 2), (p[0], p[1], 1)])
        x, y = p

    def run(code):
        nonlocal x, y
        i = 0
        while i < len(code):
            b0 = code[i]
            i += 1
            if b0 >= 32 or b0 == 28:
                if b0 == 28:
                    v = struct.unpack_from(">h", code, i)[0]; i += 2
                elif b0 <= 246:
                    v = b0 - 139
                elif b0 <= 250:
                    v = (b0 - 247) * 256 + code[i] + 108; i += 1
                elif b0 <= 254:
                    v = -(b0 - 251) * 256 - code[i] - 108; i += 1
                else:
                    v = struct.unpack_from(">i", code, i)[0] / 65536.0; i += 4
                st.append(v)
                continue
            if b0 == 12:
                b0 = 1200 + code[i]; i += 1
            if b0 in (1, 3, 18, 23):
                pop_width(expect_even=True)
                state["stems"] += len(st) // 2
                st.clear()
            elif b0 in (19, 20):
                if st:
                    pop_width(expect_even=True)
                    state["stems"] += len(st) // 2
                state["width_done"] = True
                i += (state["stems"] + 7) // 8
                st.clear()
            elif b0 == 21:
                pop_width(n=2)
                moveto(x + st[0], y + st[1]); st.clear()
            elif b0 == 22:
                pop_width(n=1)
                moveto(x + st[0], y); st.clear()
            elif b0 == 4:
                pop_width(n=1)
                moveto(x, y + st[0]); st.clear()
            elif b0 == 5:
                for k in range(0, len(st) - 1, 2):
                    line(st[k], st[k + 1])
                st.clear()
            elif b0 in (6, 7):
                h = b0 == 6
                for v in st:
                    line(v, 0) if h else line(0, v)
                    h = not h
                st.clear()
            elif b0 == 8:
                for k in range(0, len(st) - 5, 6):
                    curve(*st[k:k + 6])
                st.clear()
            elif b0 == 24:
                k = 0
                while len(st) - k >= 8:
                    curve(*st[k:k + 6]); k += 6
                line(st[k], st[k + 1]); st.clear()
            elif b0 == 25:
                k = 0
                while len(st) - k > 6:
                    line(st[k], st[k + 1]); k += 2
                curve(*st[k:k + 6]); st.clear()
            elif b0 == 27:
                k, dy1 = 0, 0
                if len(st) % 2:
                    dy1, k = st[0], 1
                while k + 4 <= len(st):
                    curve(st[k], dy1, st[k + 1], st[k + 2], st[k + 3], 0); dy1 = 0; k += 4
                st.clear()
            elif b0 == 26:
                k, dx1 = 0, 0
                if len(st) % 2:
                    dx1, k = st[0], 1
                while k + 4 <= len(st):
                    curve(dx1, st[k], st[k + 1], st[k + 2], 0, st[k + 3]); dx1 = 0; k += 4
                st.clear()
            elif b0 in (30, 31):
                h = b0 == 31
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
            elif b0 in (10, 29):
                idx = int(st.pop()) + 107
                run((lsubrs if b0 == 10 else gsubrs)[idx])
            elif b0 == 11:
                return False
            elif b0 == 14:
                pop_width(n=0)
                st.clear()
                return True
            elif b0 == 1235:                        # flex
                curve(*st[0:6]); curve(*st[6:12]); st.clear()
            elif b0 == 1234:                        # hflex
                y0 = y
                a = st
                curve(a[0], 0, a[1], a[2], a[3], 0)
                curve(a[4], 0, a[5], y0 - y, a[6], 0); st.clear()
            elif b0 == 1236:                        # hflex1
                y0 = y
                a = st
                curve(a[0], a[1], a[2], a[3], a[4], 0)
                curve(a[5], 0, a[6], a[7], a[8], y0 - (y + a[7])); st.clear()
            elif b0 == 1237:                        # flex1
                x0, y0 = x, y
                a = st
                dx = sum(a[0:10:2])
                dy = sum(a[1:10:2])
                curve(*a[0:6])
                xc = x + a[6] + a[8]
                yc = y + a[7] + a[9]
                if abs(dx) > abs(dy):
                    curve(a[6], a[7], a[8], a[9], a[10], y0 - yc)
                else:
                    curve(a[6], a[7], a[8], a[9], x0 - xc, a[10])
                st.clear()
            else:
                raise ValueError(f"operator {b0}")
        return False
    run(code)
    return contours


def build_otf(names):
    n = len(CFF_GLYPHS)
    top_fixed = lambda cso, psz, po: dict_i32(cso) + b"\x11" + dict_i32(psz) + dict_i32(po) + b"\x12"
    name_idx = cff_index([b"jfguiMiniCFF"])
    string_idx = cff_index([])
    gsubr_idx = cff_index(CFF_GSUBRS)
    hdr = bytes([1, 0, 4, 4])
    top_len = len(top_fixed(0, 0, 0))
    top_idx_len = len(cff_index([bytes(top_len)]))
    cs_off = len(hdr) + len(name_idx) + top_idx_len + len(string_idx) + len(gsubr_idx)
    cs_idx = cff_index([g["cs"] for g in CFF_GLYPHS])
    priv_off = cs_off + len(cs_idx)
    private = dict_i32(0) + b"\x14" + dict_i32(0) + b"\x15"          # defaultWidthX, nominalWidthX
    private += dict_i32(len(private) + 6) + b"\x13"                    # Subrs right after the DICT
    lsubr_idx = cff_index(CFF_LSUBRS)
    top = top_fixed(cs_off, len(private), priv_off)
    cff = hdr + name_idx + cff_index([top]) + string_idx + gsubr_idx + cs_idx + private + lsubr_idx
    # GPOS: lookup 0 = PairPos format 1 (A, V) -70; lookup 1 = extension -> PairPos format 2
    # (class 1 = {O}) x (class 1 = {A}) -30
    cov_a = struct.pack(">HHH", 1, 1, 1)
    pairset = struct.pack(">H", 1) + struct.pack(">Hh", 2, -70)
    pp1 = struct.pack(">HHHHH", 1, 12, 4, 0, 1) + struct.pack(">H", 12 + len(cov_a))
    pp1 += cov_a + pairset
    cov_o = struct.pack(">HHH", 1, 1, 3)
    cd1 = struct.pack(">HHHH", 1, 3, 1, 1)                           # format 1: start 3, [1]
    cd2 = struct.pack(">HHHHH", 2, 1, 1, 1, 1)                       # format 2: range 1..1 -> 1
    recs = struct.pack(">hhhh", 0, 0, 0, -30)                         # 2 x 2 classes
    base = 16 + len(recs)
    pp2 = struct.pack(">HHHHHHHH", 2, base, 4, 0, base + len(cov_o), base + len(cov_o) + len(cd1), 2, 2)
    pp2 += recs + cov_o + cd1 + cd2
    ext = struct.pack(">HHI", 1, 2, 8) + pp2
    lk0 = struct.pack(">HHHH", 2, 0, 1, 8) + pp1
    lk1 = struct.pack(">HHHH", 9, 0, 1, 8) + ext
    lookup_list = struct.pack(">HHH", 2, 6, 6 + len(lk0)) + lk0 + lk1
    feature = struct.pack(">HHHH", 0, 2, 0, 1)
    feature_list = struct.pack(">H", 1) + b"kern" + struct.pack(">H", 8) + feature
    langsys = struct.pack(">HHHH", 0, 0xFFFF, 1, 0)
    script = struct.pack(">HH", 4, 0) + langsys
    script_list = struct.pack(">H", 1) + b"DFLT" + struct.pack(">H", 8) + script
    gpos = struct.pack(">HHHHH", 1, 0, 10, 10 + len(script_list), 10 + len(script_list) + len(feature_list))
    gpos += script_list + feature_list + lookup_list
    ctrs = [t2_eval(g["cs"], CFF_LSUBRS, CFF_GSUBRS) for g in CFF_GLYPHS]
    for g, c in zip(CFF_GLYPHS, ctrs):
        pts = [p for cc in c for p in cc]
        g["contours"] = c
        g["bbox"] = (min(p[0] for p in pts), min(p[1] for p in pts), max(p[0] for p in pts), max(p[1] for p in pts))
    fb = tuple(int(v) for v in (min(g["bbox"][0] for g in CFF_GLYPHS), min(g["bbox"][1] for g in CFF_GLYPHS),
                                max(g["bbox"][2] for g in CFF_GLYPHS), max(g["bbox"][3] for g in CFF_GLYPHS)))
    tables = {
        "CFF ": cff,
        "GPOS": gpos,
        "head": head_table(1000, fb, 0, 0),
        "hhea": hhea_table(850, -250, 50, 1000, n),
        "maxp": struct.pack(">IH", 0x5000, n),
        "hmtx": b"".join(struct.pack(">Hh", g["adv"], int(g["bbox"][0])) for g in CFF_GLYPHS),
        "cmap": cmap12(OTF_CMAP),
        "OS/2": os2_table(400, 0x40, 480, 700, 280, 60, 800, -200, 100),
        "post": post_table(-120, 60),
        "name": name_table(names),
    }
    return sfnt(tables, 0x4F54544F)


def cmap_mac0(mapping):
    """(1,0) format 0 from {mac byte: gid}"""
    arr = bytes(mapping.get(i, 0) for i in range(256))
    sub = struct.pack(">HHH", 0, 6 + 256, 0) + arr
    return struct.pack(">HHHHI", 0, 1, 1, 0, 12) + sub


MAC_GLYPHS = [dict(TT_GLYPHS[i]) for i in (0, 1, 2, 3, 4, 8)]


def build_mac_ttf():
    glyphs = MAC_GLYPHS
    glyf, loca = b"", []
    for g in glyphs:
        loca.append(len(glyf))
        d, bbox = encode_simple(g["contours"])
        g["bbox"] = bbox
        glyf += d + bytes((-len(d)) % 4)
    loca.append(len(glyf))
    n = len(glyphs)
    hmtx = b"".join(struct.pack(">Hh", g["adv"], g.get("lsb", g["bbox"][0])) for g in glyphs)
    # Apple kern version 1: u32 version, u32 nTables, subtable {u32 length, u16 coverage,
    # u16 tupleIndex, format 0 pairs}
    pairs = [(1, 2, -55), (2, 1, -44)]
    body = struct.pack(">HHHH", len(pairs), 12, 1, 0) + b"".join(struct.pack(">HHh", *p) for p in pairs)
    sub = struct.pack(">IHH", 8 + len(body), 0x0000, 0) + body
    kern = struct.pack(">II", 0x00010000, 1) + sub
    tables = {
        "head": head_table(1000, (0, -250, 1000, 900), 0, 1),
        "hhea": hhea_table(850, -250, 50, 1000, n),
        "maxp": struct.pack(">IHHHHHHHHHHHHHH", 0x10000, n, 16, 4, 16, 4, 2, 0, 0, 0, 0, 0, 0, 2, 2),
        "hmtx": hmtx,
        "loca": b"".join(struct.pack(">I", o) for o in loca),
        "glyf": glyf,
        "cmap": cmap_mac0(MAC_CMAP),
        "OS/2": os2_table(400, 0x40, 500, 700, 300, 50, 800, -200, 100),
        "post": post_table(-100, 50),
        "name": name_table([(1, 0x409, "jfgui Mac"), (2, 0x409, "Regular")]),
        "kern": kern,
    }
    return sfnt(tables)


MAC_CMAP = {0x41: 1, 0x56: 2, 0x4F: 3, 0x48: 4, 0x80: 5}    # 0x80 = Mac Roman 'Ä'

CID_LSUBRS = [[cs(0, 500, "rlineto", "return")], [cs(500, 0, "rlineto", "return")]]
CID_GLYPHS = [
    dict(adv=500, fd=0, cs=cs(50, 0, "rmoveto", 400, 700, -400, "hlineto", "endchar")),
    dict(adv=600, fd=0, cs=cs(100, 100, "rmoveto", -107, "callsubr", 300, "hlineto", "endchar")),
    dict(adv=700, fd=1, cs=cs(100, 100, "rmoveto", -107, "callsubr", 300, "vlineto", "endchar")),
]


def build_cid_otf():
    hdr = bytes([1, 0, 4, 4])
    name_idx = cff_index([b"jfguiCID"])
    string_idx = cff_index([b"Adobe", b"Identity"])
    gsubr_idx = cff_index([])
    top_fixed = lambda cso, fda, fds: (dict_i32(391) + dict_i32(392) + dict_i32(0) + b"\x0c\x1e" +
                                       dict_i32(cso) + b"\x11" + dict_i32(fda) + b"\x0c\x24" +
                                       dict_i32(fds) + b"\x0c\x25")
    top_len = len(top_fixed(0, 0, 0))
    pos = len(hdr) + len(name_idx) + len(cff_index([bytes(top_len)])) + len(string_idx) + len(gsubr_idx)
    cs_idx = cff_index([g["cs"] for g in CID_GLYPHS])
    cs_off = pos
    pos += len(cs_idx)
    fdselect = struct.pack(">BH", 3, 2) + struct.pack(">HB", 0, 0) + struct.pack(">HB", 2, 1) + \
        struct.pack(">H", len(CID_GLYPHS))
    fds_off = pos
    pos += len(fdselect)
    # private DICTs + their subrs, then the FDArray (font DICTs with Private [size, offset])
    privs, blobs = [], b""
    fd_len = len(cff_index([bytes(len(dict_i32(0) * 2) + 1)] * 2))
    fda_off = pos
    pos += fd_len
    for k in range(2):
        priv = dict_i32(0) + b"\x14"
        priv += dict_i32(len(priv) + 6) + b"\x13"
        sub = cff_index(CID_LSUBRS[k])
        privs.append((pos + len(blobs), len(priv)))
        blobs += priv + sub
    fda = cff_index([dict_i32(sz) + dict_i32(off) + b"\x12" for off, sz in privs])
    assert len(fda) == fd_len
    top = top_fixed(cs_off, fda_off, fds_off)
    cff = hdr + name_idx + cff_index([top]) + string_idx + gsubr_idx + cs_idx + fdselect + fda + blobs
    for g in CID_GLYPHS:
        c = t2_eval(g["cs"], CID_LSUBRS[g["fd"]], [])
        pts = [p for cc in c for p in cc]
        g["contours"] = c
        g["bbox"] = (min(p[0] for p in pts), min(p[1] for p in pts), max(p[0] for p in pts), max(p[1] for p in pts))
    n = len(CID_GLYPHS)
    tables = {
        "CFF ": cff,
        "head": head_table(1000, (0, 0, 1000, 800), 0, 0),
        "hhea": hhea_table(850, -250, 50, 700, n),
        "maxp": struct.pack(">IH", 0x5000, n),
        "hmtx": b"".join(struct.pack(">Hh", g["adv"], int(g["bbox"][0])) for g in CID_GLYPHS),
        "cmap": cmap12({0x41: 1, 0x42: 2}),
        "OS/2": os2_table(400, 0x40, 500, 700, 300, 50, 800, -200, 100),
        "post": post_table(-100, 50),
        "name": name_table([(1, 0x409, "jfgui CID"), (2, 0x409, "Regular")]),
    }
    return sfnt(tables, 0x4F54544F)


# ------------------------------------------------------------------ expected output
def fmt_outline(key, gid, adv, contours):
    """same text as fonttest's put_outline: G line, P lines (x 1024), E per contour"""
    pts = [p for c in contours for p in c]
    if pts:
        box = (math.floor(min(p[0] for p in pts)), math.floor(min(p[1] for p in pts)),
               math.ceil(max(p[0] for p in pts)), math.ceil(max(p[1] for p in pts)))
        n = len(contours)
    else:
        box, n = (0, 0, 0, 0), 0
    out = f"G {key} {gid} {adv} {n} {box[0]} {box[1]} {box[2]} {box[3]}\n"
    for c in contours:
        for x, y, tag in c:
            out += f"P {tag} {int(round(x * 1024))} {int(round(y * 1024))}\n"
        out += "E\n"
    return out


def tt_contours_for(glyphs, gid):
    c = resolve(glyphs, gid)
    g = glyphs[gid]
    shift = g.get("lsb", g["bbox"][0]) - g["bbox"][0]
    return [[(x + shift, y, on) for x, y, on in cc] for cc in c if cc]


def main():
    FIX.mkdir(parents=True, exist_ok=True)
    tt_names = [(1, 0x409, "jfgui Mini"), (2, 0x409, "Regular"), (2, 0x407, "Standard"),
                (4, 0x409, "jfgui Mini Regular")]
    ttf = build_ttf(TT_GLYPHS, tt_names)
    (FIX / "mini.ttf").write_bytes(ttf)
    bold_names = [(1, 0x409, "jfgui Mini"), (2, 0x409, "Bold"), (4, 0x409, "jfgui Mini Bold"),
                  (16, 0x409, "jfgui Mini Typo")]
    bold = build_ttf(TT_GLYPHS, bold_names, weight=700, fs_sel=0x20, macstyle=1, colr=False, kern=None)
    (FIX / "mini.ttc").write_bytes(ttc([ttf, bold]))
    otf_names = [(1, 0x409, "jfgui Mini CFF"), (2, 0x409, "Italic"), (4, 0x409, "jfgui Mini CFF Italic")]
    otf = build_otf(otf_names)
    # italic flag through fsSelection
    (FIX / "mini.otf").write_bytes(otf)
    import font_check
    font_check.make_cbdt_font(FIX / "cbdt.ttf")
    (FIX / "mac.ttf").write_bytes(build_mac_ttf())
    (FIX / "cid.otf").write_bytes(build_cid_otf())

    exp = ""
    # ---- TrueType (also face 0 of the collection)
    for path, face, names, glyphs, cmap, kern, flags, weight, layers in (
            ("mini.ttf", 0, ("jfgui Mini", "Standard", "jfgui Mini Regular", "jfgui Mini", "Standard"),
             TT_GLYPHS, TT_CMAP, TT_KERN, 0x1000 | 8, 400, True),
            ("mini.ttc", 1, ("jfgui Mini", "Bold", "jfgui Mini Bold", "jfgui Mini Typo", "Bold"),
             TT_GLYPHS, TT_CMAP, {}, 2, 700, False)):
        exp += f"font {path} {face}\n"
        for nid, v in zip((1, 2, 4, 16, 17), names):
            exp += f"name {nid} {v}\n"
        exp += f"metrics 1000 850 -250 50 500 700 -100 50 300 50 {weight} {flags:x} {len(glyphs)}\n"
        for cp in (0x41, 0x56, 0x4F, 0xC4, 0x53, 0x43, 0x20, 0x42, 0x1F600):
            exp += f"cmap {cp} {cmap.get(cp, 0)}\n"
        for gid in range(len(glyphs)):
            exp += fmt_outline(f"g{gid}", gid, glyphs[gid]["adv"], tt_contours_for(glyphs, gid))
        for a, b in ((0x41, 0x56), (0x56, 0x41), (0x4F, 0x41), (0x41, 0x4F)):
            ga, gb = cmap[a], cmap[b]
            v = kern.get((ga, gb), 0)
            exp += f"kern {a} {b} {v} {v}\n"
        if layers:
            exp += "layers 8 2 9 ffe04020 8000000a 0\n"
        else:
            exp += "layers 8 0\n"
    # ---- CFF
    exp += "font mini.otf 0\n"
    for nid, v in zip((1, 2, 4, 16, 17), ("jfgui Mini CFF", "Italic", "jfgui Mini CFF Italic",
                                          "jfgui Mini CFF", "Italic")):
        exp += f"name {nid} {v}\n"
    exp += f"metrics 1000 850 -250 50 480 700 -120 60 280 60 400 {0x1000 | 16:x} {len(CFF_GLYPHS)}\n"
    for cp in (0x41, 0x56, 0x4F, 0x42, 0x46, 0x47, 0x4A, 0x51, 0x1F600, 0x43):
        exp += f"cmap {cp} {OTF_CMAP.get(cp, 0)}\n"
    for gid, g in enumerate(CFF_GLYPHS):
        exp += fmt_outline(f"g{gid}", gid, g["adv"], g["contours"])
    for a, b, v in ((0x41, 0x56, -70), (0x56, 0x41, 0), (0x4F, 0x41, -30), (0x41, 0x4F, 0)):
        exp += f"kern {a} {b} {v} 0\n"
    exp += "layers 3 0\n"
    # ---- Mac Roman cmap + Apple kern
    mac = MAC_GLYPHS
    exp += "font mac.ttf 0\n"
    for nid, v in zip((1, 2, 4, 16, 17), ("jfgui Mac", "Regular", "", "jfgui Mac", "Regular")):
        exp += f"name {nid} {v}\n"
    exp += f"metrics 1000 850 -250 50 500 700 -100 50 300 50 400 {0x1000:x} {len(mac)}\n"
    for cp, g in ((0x41, 1), (0x56, 2), (0x4F, 3), (0xC4, 5), (0x48, 4), (0x42, 0), (0x1F600, 0)):
        exp += f"cmap {cp} {g}\n"
    for gid in range(len(mac)):
        exp += fmt_outline(f"g{gid}", gid, mac[gid]["adv"], tt_contours_for(mac, gid))
    for a, b, v in ((0x41, 0x56, -55), (0x56, 0x41, -44), (0x4F, 0x41, 0), (0x41, 0x4F, 0)):
        exp += f"kern {a} {b} {v} {v}\n"
    exp += "layers 1 0\n"
    # ---- CID-keyed CFF
    exp += "font cid.otf 0\n"
    for nid, v in zip((1, 2, 4, 16, 17), ("jfgui CID", "Regular", "", "jfgui CID", "Regular")):
        exp += f"name {nid} {v}\n"
    exp += f"metrics 1000 850 -250 50 500 700 -100 50 300 50 400 {16:x} {len(CID_GLYPHS)}\n"
    for cp, g in ((0x41, 1), (0x42, 2), (0x43, 0), (0x1F600, 0), (0x20, 0), (0x56, 0), (0x4F, 0)):
        exp += f"cmap {cp} {g}\n"
    for gid, g in enumerate(CID_GLYPHS):
        exp += fmt_outline(f"g{gid}", gid, g["adv"], g["contours"])
    for a, b in ((0x41, 0x56), (0x56, 0x41), (0x4F, 0x41), (0x41, 0x4F)):
        exp += f"kern {a} {b} 0 0\n"
    exp += "layers 1 0\n"
    (FIX / "expected.txt").write_text(exp, encoding="utf-8", newline="\n")
    print("wrote", FIX)


if __name__ == "__main__":
    main()
