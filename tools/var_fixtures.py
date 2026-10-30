#!/usr/bin/env python3
"""var_fixtures.py -- synthetic variable fonts for `fonttest --selftest` and the font dialog
(apps/fonttest/tests/varfont, apps/fontpicktest/tests/var*.uis).

Writes apps/fonttest/tests/varfont/:
  jfvar.ttf          "jfgui Var": axes wght 100..400..900 (avar: 0.5 -> 0.25), wdth 50..100,
                     slnt -12..0, a hidden custom axis TEST 0..10; five named instances
                     (German and English names); gvar with embedded and shared peak tuples,
                     an intermediate region, shared and private packed point numbers, IUP over
                     one and two contours, an all-off-curve contour, a composite with
                     USE_MY_METRICS and a varied component offset, short gvar offsets; HVAR
                     with an advance mapping (word + byte deltas); MVAR (hasc, unds, xhgt; a
                     LONG_WORDS item data)
  jfvar-phantom.ttf  "jfgui Var Phantom": wght 200..400..800 and opsz 8..12..48, named
                     instances that differ in opsz, no HVAR / MVAR / avar: advances and the
                     origin from the phantom points (pp1 and pp2 deltas), word and long packed
                     deltas, zero runs, word point numbers, a scaled component with
                     SCALED_COMPONENT_OFFSET, long gvar offsets
  Both map every letter and digit to one of their glyphs (a preview shows them varied).
  budget.bin         (not a font extension: font scans skip it) a glyph of 8000 points with
                     4095 tuples of one private point each: more work than VAR_BUDGET, so
                     the glyph comes out empty at an instance (and stays quick)
  expected.txt       what fonttest --selftest prints for these instances, computed here from
                     the deltas (independent of the Rust code): coordinates, outlines,
                     advances, MVAR metrics

Only synthetic geometry: the files may be redistributed freely.
  python tools/var_fixtures.py
"""
import math
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from font_fixtures import (cmap4, encode_composite, encode_simple, fmt_outline, head_table, hhea_table,  # noqa: E402
                           name_table, os2_table, post_table, sfnt)

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "apps" / "fonttest" / "tests" / "varfont"
F2 = 16384


# ------------------------------------------------------------------ packed data
def packed_points(pts, words=False):
    """packed point numbers (None: every point)"""
    if pts is None:
        return b"\0"
    n = len(pts)
    out = bytes([n]) if n < 128 else bytes([0x80 | (n >> 8), n & 0xFF])
    prev, i = 0, 0
    while i < n:
        run = pts[i:i + (64 if words else 128)]
        out += bytes([(len(run) - 1) | (0x80 if words else 0)])
        for p in run:
            d = p - prev
            prev = p
            out += struct.pack(">H", d) if words else bytes([d])
        i += len(run)
    return out


def packed_deltas(vals, force=None):
    """packed deltas: zero runs, bytes, words or longs (force: 'w' / 'l' for non-zero runs)"""
    out, i = b"", 0
    while i < len(vals):
        v = vals[i]
        j = i
        if v == 0:
            while j < len(vals) and vals[j] == 0 and j - i < 64:
                j += 1
            out += bytes([0x80 | (j - i - 1)])
        else:
            def kind(x):
                if force:
                    return force
                return "b" if -128 <= x <= 127 else ("w" if -32768 <= x <= 32767 else "l")
            k = kind(v)
            while j < len(vals) and vals[j] != 0 and kind(vals[j]) == k and j - i < 64:
                j += 1
            run = vals[i:j]
            if k == "b":
                out += bytes([len(run) - 1]) + b"".join(struct.pack(">b", x) for x in run)
            elif k == "w":
                out += bytes([0x40 | (len(run) - 1)]) + b"".join(struct.pack(">h", x) for x in run)
            else:
                out += bytes([0xC0 | (len(run) - 1)]) + b"".join(struct.pack(">i", x) for x in run)
        i = j
    return out


def f2(v):
    return int(round(v * F2))


# ------------------------------------------------------------------ tables
def fvar_table(axes, instances):
    """axes [(tag, min, def, max, flags, nameid)], instances [(nameid, [coords])]"""
    n = len(axes)
    t = struct.pack(">HHHHHHHH", 1, 0, 16, 2, n, 20, len(instances), 4 + 4 * n)
    for tag, mn, df, mx, fl, nid in axes:
        t += tag.encode() + struct.pack(">iiiHH", int(mn * 65536), int(df * 65536), int(mx * 65536), fl, nid)
    for nid, coords in instances:
        t += struct.pack(">HH", nid, 0) + b"".join(struct.pack(">i", int(c * 65536)) for c in coords)
    return t


def avar_table(maps):
    t = struct.pack(">HHHH", 1, 0, 0, len(maps))
    for m in maps:
        t += struct.pack(">H", len(m)) + b"".join(struct.pack(">hh", f2(a), f2(b)) for a, b in m)
    return t


def gvar_table(naxes, shared, glyphs, long_off):
    """shared: [peak tuple]; glyphs: per glyph None or dict(shared_points, tuples=[dict(peak |
    shared, inter, points, dx, dy, force)])"""
    datas = []
    for gv in glyphs:
        if not gv:
            datas.append(b"")
            continue
        hdrs, ser = b"", b""
        sp = gv.get("shared_points", "none")
        if sp != "none":
            ser += packed_points(sp, gv.get("words", False))
        for tu in gv["tuples"]:
            flags = 0
            extra = b""
            if "shared" in tu:
                flags |= tu["shared"]
            else:
                flags |= 0x8000
                extra += b"".join(struct.pack(">h", f2(v)) for v in tu["peak"])
            if tu.get("inter"):
                flags |= 0x4000
                st, en = tu["inter"]
                extra += b"".join(struct.pack(">h", f2(v)) for v in st) + b"".join(struct.pack(">h", f2(v)) for v in en)
            body = b""
            if "points" in tu or sp == "none":
                # private numbers; every point without shared ones (an explicit count 0:
                # FreeType reads a tuple with neither as no points, HarfBuzz as every point)
                flags |= 0x2000
                body += packed_points(tu.get("points"), tu.get("words", False))
            body += packed_deltas(tu["dx"], tu.get("force")) + packed_deltas(tu["dy"], tu.get("force"))
            hdrs += struct.pack(">HH", len(body), flags) + extra
            ser += body
        count = len(gv["tuples"]) | (0x8000 if sp != "none" else 0)
        d = struct.pack(">HH", count, 4 + len(hdrs)) + hdrs + ser
        datas.append(d + bytes(len(d) % 2))
    st = b"".join(struct.pack(">h", f2(v)) for t in shared for v in t)
    offs, o = [], 0
    for d in datas:
        offs.append(o)
        o += len(d)
    offs.append(o)
    ot = b"".join(struct.pack(">I", x) for x in offs) if long_off else b"".join(struct.pack(">H", x // 2) for x in offs)
    so = 20 + len(ot)
    do = so + len(st)
    return struct.pack(">HHHHIHHI", 1, 0, naxes, len(shared), so, len(glyphs), 1 if long_off else 0, do) + ot + st + b"".join(datas)


def ivs(naxes, regions, datas):
    """item variation store: regions [[(start, peak, end)] per axis]; datas [(rows, words, long)]
    rows: [[delta per region index]] over every region (region index list = all regions)"""
    rl = struct.pack(">HH", naxes, len(regions)) + b"".join(
        struct.pack(">hhh", f2(s), f2(p), f2(e)) for r in regions for s, p, e in r)
    blobs = []
    for rows, words, long in datas:
        nr = len(regions)
        b = struct.pack(">HHH", len(rows), words | (0x8000 if long else 0), nr) + b"".join(struct.pack(">H", i) for i in range(nr))
        for row in rows:
            for k, v in enumerate(row):
                if k < words:
                    b += struct.pack(">i" if long else ">h", v)
                else:
                    b += struct.pack(">h" if long else ">b", v)
        blobs.append(b)
    hdr = 8 + 4 * len(blobs)
    off = hdr + len(rl)
    offs = []
    for b in blobs:
        offs.append(off)
        off += len(b)
    return struct.pack(">HIH", 1, hdr, len(blobs)) + b"".join(struct.pack(">I", o) for o in offs) + rl + b"".join(blobs)


def hvar_table(store, mapping):
    """advance mapping: format 0, 2-byte entries, 8 inner bits"""
    m = struct.pack(">BBH", 0, 0x10 | 7, len(mapping)) + b"".join(struct.pack(">H", (o << 8) | i) for o, i in mapping)
    return struct.pack(">HHIIII", 1, 0, 20, 20 + len(store), 0, 0) + store + m


def mvar_table(records, store):
    recs = sorted(records)
    hdr = 12 + 8 * len(recs)
    return (struct.pack(">HHHHHH", 1, 0, 0, 8, len(recs), hdr) + b"".join(t.encode() + struct.pack(">HH", o, i) for t, o, i in recs)
            + store)


def build(glyphs, cmap, names, axes, extra, weight=400):
    glyf, loca, bboxes = b"", [], []
    for g in glyphs:
        loca.append(len(glyf))
        if "comps" in g:
            d = encode_composite_flags(g["comps"], g["bbox"])
            bbox = g["bbox"]
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
        "head": head_table(1000, fb, 0, 1),
        "hhea": hhea_table(850, -250, 50, max(g["adv"] for g in glyphs), n),
        "maxp": struct.pack(">IHHHHHHHHHHHHHH", 0x10000, n, 8000, 4, 16, 4, 2, 0, 0, 0, 0, 0, 0, 2, 2),
        "hmtx": hmtx,
        "loca": b"".join(struct.pack(">I", o) for o in loca),
        "glyf": glyf,
        "cmap": cmap4(cmap),
        "OS/2": os2_table(weight, 0x40, 500, 700, 300, 50, 800, -200, 100),
        "post": post_table(-100, 50),
        "name": name_table(names),
        "fvar": fvar_table(axes, extra.pop("instances", [])),
    }
    tables.update(extra)
    return sfnt(tables)


def encode_composite_flags(comps, bbox):
    """comps: [(gid, dx, dy, scale, extra flags)] (encode_composite plus USE_MY_METRICS /
    SCALED_COMPONENT_OFFSET)"""
    data = struct.pack(">hhhhh", -1, *bbox)
    for i, (gid, dx, dy, scale, xf) in enumerate(comps):
        fl = 0x02 | 0x01 | xf
        if scale is not None:
            fl |= 0x08
        if i + 1 < len(comps):
            fl |= 0x20
        data += struct.pack(">HHhh", fl, gid, dx, dy)
        if scale is not None:
            data += struct.pack(">h", int(round(scale * 16384)))
    return data


# ------------------------------------------------------------------ the model (expected values)
def normalize(axes, avar, user):
    out = []
    for k, (tag, mn, df, mx, fl, nid) in enumerate(axes):
        v = min(max(user.get(tag, df), mn), mx)
        x = 0.0 if v == df else ((v - df) / (df - mn) if v < df else (v - df) / (mx - df))
        n = math.floor(x * 65536 + 0.5)
        if avar:
            seg = [(f2(a) * 4, f2(b) * 4) for a, b in avar[k]]
            if n <= seg[0][0]:
                n = n - seg[0][0] + seg[0][1]
            else:
                i = 1
                while i < len(seg) - 1 and n > seg[i][0]:
                    i += 1
                if n >= seg[i][0]:
                    n = n - seg[i][0] + seg[i][1]
                else:
                    n = math.floor(seg[i - 1][1] + (seg[i][1] - seg[i - 1][1]) * (n - seg[i - 1][0]) / (seg[i][0] - seg[i - 1][0]) + .5)
        out.append((n + 2) >> 2)
    return out


def scalar(coords, peak, inter):
    s = 1.0
    for i, p in enumerate(peak):
        p = f2(p)
        if p == 0:
            continue
        v = coords[i]
        if v == 0:
            return 0.0
        if v == p:
            continue
        if inter:
            st, en = f2(inter[0][i]), f2(inter[1][i])
            if v < st or v > en:
                return 0.0
            s *= (v - st) / (p - st) if v < p else (en - v) / (en - p)
        else:
            if v < min(0, p) or v > max(0, p):
                return 0.0
            s *= v / p
    return s


def iup(orig, ends, touched, d):
    start = 0
    for e in ends:
        idx = [i for i in range(start, e + 1) if touched[i]]
        if idx:
            for i in range(start, e + 1):
                if touched[i]:
                    continue
                # nearest touched before / after (cyclic)
                prv = max([t for t in idx if t < i], default=idx[-1])
                nxt = min([t for t in idx if t > i], default=idx[0])
                for ax in (0, 1):
                    c1, d1, c2, d2 = orig[prv][ax], d[prv][ax], orig[nxt][ax], d[nxt][ax]
                    if c1 > c2:
                        c1, d1, c2, d2 = c2, d2, c1, d1
                    c = orig[i][ax]
                    if c1 == c2:
                        v = d1 if d1 == d2 else 0.0
                    elif c <= c1:
                        v = d1
                    elif c >= c2:
                        v = d2
                    else:
                        v = d1 + (c - c1) * (d2 - d1) / (c2 - c1)
                    d[i][ax] = v
        start = e + 1


def deltas(gv, shared, coords, orig, ends, npts):
    out = [[0.0, 0.0] for _ in range(npts)]
    if not gv:
        return out
    for tu in gv["tuples"]:
        peak = shared[tu["shared"]] if "shared" in tu else tu["peak"]
        s = scalar(coords, peak, tu.get("inter"))
        if s == 0:
            continue
        pts = tu["points"] if "points" in tu else gv.get("shared_points")
        if pts is None or pts == "none":
            for i in range(npts):
                out[i][0] += tu["dx"][i] * s
                out[i][1] += tu["dy"][i] * s
            continue
        tmp = [[0.0, 0.0] for _ in range(npts)]
        touched = [False] * npts
        for k, p in enumerate(pts):
            tmp[p][0] += tu["dx"][k] * s
            tmp[p][1] += tu["dy"][k] * s
            touched[p] = True
        if ends:
            iup(orig, ends, touched, tmp)
        for i in range(npts):
            out[i][0] += tmp[i][0]
            out[i][1] += tmp[i][1]
    return out


def outline(glyphs, gvars, shared, coords, gid):
    """(contours [[(x, y, on)]], pp deltas) of glyph at coords, components resolved"""
    g = glyphs[gid]
    if "comps" in g:
        n = len(g["comps"])
        dl = deltas(gvars[gid], shared, coords, [], [], n + 4)
        res, pp = [], (dl[n][0], dl[n + 1][0])
        for k, (cg, dx, dy, scale, xf) in enumerate(g["comps"]):
            sub, spp = outline(glyphs, gvars, shared, coords, cg)
            if xf & 0x200:
                pp = spp
            s = 1.0 if scale is None else round(scale * 16384) / 16384.0
            ox, oy = dx + dl[k][0], dy + dl[k][1]
            if scale is not None and xf & 0x800:
                ox, oy = ox * s, oy * s
            res += [[(x * s + ox, y * s + oy, on) for x, y, on in c] for c in sub]
        return res, pp
    pts = [(x, y, on) for c in g["contours"] for x, y, on in c]
    ends, k = [], 0
    for c in g["contours"]:
        k += len(c)
        ends.append(k - 1)
    n = len(pts)
    dl = deltas(gvars[gid], shared, coords, [(x, y) for x, y, _ in pts], ends, n + 4)
    out, k = [], 0
    for c in g["contours"]:
        cc = []
        for x, y, on in c:
            cc.append((x + dl[k][0], y + dl[k][1], on))
            k += 1
        out.append(cc)
    return out, (dl[n][0], dl[n + 1][0])


def ivs_delta(regions, row, coords):
    s = 0.0
    for r, dv in zip(regions, row):
        sc = 1.0
        for a, (st, pk, en) in enumerate(r):
            st, pk, en = f2(st), f2(pk), f2(en)
            v = coords[a] if a < len(coords) else 0
            if pk == 0 or v == pk:
                continue
            if v == 0 or v <= st or v >= en:
                sc = 0.0
                break
            sc *= (v - st) / (pk - st) if v < pk else (en - v) / (en - pk)
        s += sc * dv
    return s


# ------------------------------------------------------------------ jfvar.ttf
AXES = [("wght", 100, 400, 900, 0, 256), ("wdth", 50, 100, 100, 0, 257), ("slnt", -12, 0, 0, 0, 258),
        ("TEST", 0, 0, 10, 1, 259)]
AVAR = [[(-1, -1), (0, 0), (0.5, 0.25), (1, 1)], [(-1, -1), (0, 0), (1, 1)], [(-1, -1), (0, 0), (1, 1)],
        [(-1, -1), (0, 0), (1, 1)]]
INSTANCES = [(260, [100, 100, 0, 0]), (261, [400, 100, 0, 0]), (262, [700, 100, 0, 0]), (263, [900, 75, 0, 0]),
             (264, [400, 100, -12, 0])]
NAMES = [(1, 0x409, "jfgui Var"), (2, 0x409, "Regular"), (2, 0x407, "Standard"), (4, 0x409, "jfgui Var Regular"),
         (256, 0x409, "Weight"), (256, 0x407, "Strichstärke"), (257, 0x409, "Width"), (257, 0x407, "Breite"),
         (258, 0x409, "Slant"), (259, 0x409, "Test axis"), (260, 0x409, "Thin"), (260, 0x407, "Dünn"),
         (261, 0x409, "Regular"), (261, 0x407, "Standard"), (262, 0x409, "Bold"), (262, 0x407, "Fett"),
         (263, 0x409, "Condensed Black"), (264, 0x409, "Italic"), (264, 0x407, "Kursiv")]
GLYPHS = [
    dict(adv=600, contours=[[(50, 0, 1), (50, 700, 1), (550, 700, 1), (550, 0, 1)]]),                    # 0 .notdef
    dict(adv=250, contours=[]),                                                                          # 1 space
    dict(adv=1000, contours=[[(100, 0, 1), (400, 700, 1), (600, 700, 1), (900, 0, 1), (700, 0, 1),        # 2 A
                              (500, 500, 0), (300, 0, 1)]]),
    dict(adv=700, contours=[[(100, 0, 1), (100, 500, 1), (350, 700, 1), (600, 500, 1), (600, 0, 1)],      # 3 B
                            [(250, 150, 1), (450, 150, 1), (350, 350, 1)]]),
    dict(adv=1000, comps=[(2, 0, 0, None, 0x200), (5, 300, 800, None, 0)], bbox=(100, 0, 900, 900)),       # 4 Adieresis
    dict(adv=0, contours=[[(0, 0, 1), (0, 100, 1), (100, 100, 1), (100, 0, 1)]]),                        # 5 dot
    dict(adv=900, contours=[[(100, 700, 1), (800, 700, 1), (450, 0, 1)]]),                               # 6 V (no gvar)
    dict(adv=900, contours=[[(450, 700, 0), (800, 700, 0), (800, 0, 0), (100, 0, 0), (100, 700, 0)]]),     # 7 O
]
CMAP = {0x20: 1, 0x41: 2, 0x42: 3, 0xC4: 4, 0x2E: 5, 0x56: 6, 0x4F: 7}
LETTERS = list(range(0x30, 0x3A)) + list(range(0x41, 0x5B)) + list(range(0x61, 0x7B))
for _i, _c in enumerate(LETTERS):
    CMAP.setdefault(_c, (2, 3, 6, 7)[_i % 4])
SHARED = [[-1, 0, 0, 0], [0, -1, 0, 0]]
GVARS = [
    None,
    dict(tuples=[dict(peak=[1, 0, 0, 0], dx=[0, 0, 0, 0], dy=[0, 0, 0, 0])]),                            # space: zero deltas
    dict(shared_points=[0, 3, 6], tuples=[
        dict(peak=[1, 0, 0, 0], points=None, dx=[-40, -40, 40, 40, 40, 0, -40, 0, 0, 0, 0],
             dy=[0, 0, 0, 0, 0, -60, 0, 0, 0, 0, 0]),
        dict(shared=0, points=[0, 3], dx=[30, -30], dy=[0, 0]),                                          # IUP
        dict(peak=[0, -1, 0, 0], inter=([0, -1, 0, 0], [0, 0, 0, 0]), points=None,                         # all points
             dx=[0] * 11, dy=[0] * 11),
        dict(peak=[0, -0.5, 0, 0], inter=([0, -1, 0, 0], [0, 0, 0, 0]), points=None,
             dx=[-100, -50, 50, 100, 60, 0, -60, 0, 0, 0, 0], dy=[0] * 11),
        dict(shared=1, dx=[-50, 50, 0], dy=[0, 0, 20]),                                                  # shared points
        dict(peak=[0, 0, -1, 0], points=None, dx=[0, 140, 140, 0, 0, 100, 0, 0, 0, 0, 0], dy=[0] * 11),
        dict(peak=[1, -1, 0, 0], points=None, dx=[10] * 7 + [0] * 4, dy=[0] * 11),
        dict(peak=[0, 0, 0, 1], points=None, dx=[0] * 11, dy=[100] * 7 + [0] * 4),
    ]),
    dict(tuples=[                                                                                        # B: two contours
        dict(peak=[1, 0, 0, 0], points=[0, 2, 6], dx=[-20, 40, -10], dy=[0, 20, 0]),
        dict(peak=[-1, 0, 0, 0], points=[0, 1], dx=[16, 16], dy=[0, -32]),
    ]),
    dict(tuples=[dict(peak=[1, 0, 0, 0], points=[1], dx=[20], dy=[40]),                                  # Adieresis
                 dict(peak=[-1, 0, 0, 0], dx=[0, -10, 0, 0, 0, 0], dy=[0, 0, 0, 0, 0, 0])]),
    dict(tuples=[dict(peak=[1, 0, 0, 0], dx=[-10, -10, 10, 10, 0, 0, 0, 0], dy=[-10, 10, 10, -10, 0, 0, 0, 0])]),
    None,
    dict(tuples=[dict(peak=[1, 0, 0, 0], dx=[0, 40, 40, -40, -40, 0, 0, 0, 0], dy=[20, 20, -20, -20, 20, 0, 0, 0, 0])]),
]
# HVAR: regions wght+, wght-, wdth-; rows per item (glyph mapping -> item)
REGIONS = [[(0, 1, 1), (0, 0, 0), (0, 0, 0), (0, 0, 0)], [(-1, -1, 0), (0, 0, 0), (0, 0, 0), (0, 0, 0)],
           [(0, 0, 0), (-1, -1, 0), (0, 0, 0), (0, 0, 0)]]
HV_ROWS = [[0, 0, 0], [50, -20, -100], [300, -40, -100], [80, 0, -50]]     # word, byte, byte
HV_MAP = [(0, 0), (0, 1), (0, 2), (0, 3), (0, 2), (0, 0), (0, 0), (0, 3)]
MV_ROWS = [[50, 0, 0], [10, 0, 0], [30, 0, -20]]                         # long data: i32, i16, i16
MV_RECS = [("hasc", 0, 0), ("unds", 0, 1), ("xhgt", 0, 2)]

# ------------------------------------------------------------------ jfvar-phantom.ttf
P_AXES = [("wght", 200, 400, 800, 0, 256), ("opsz", 8, 12, 48, 0, 257)]
P_NAMES = [(1, 0x409, "jfgui Var Phantom"), (2, 0x409, "Regular"), (4, 0x409, "jfgui Var Phantom Regular"),
           (256, 0x409, "Weight"), (257, 0x409, "Optical size"), (258, 0x409, "Caption"), (259, 0x409, "Regular"),
           (260, 0x409, "Display"), (261, 0x409, "Display Bold")]
P_INSTANCES = [(258, [400, 8]), (259, [400, 12]), (260, [400, 48]), (261, [800, 48])]
P_GLYPHS = [
    dict(adv=600, contours=[[(50, 0, 1), (50, 700, 1), (550, 700, 1), (550, 0, 1)]]),
    dict(adv=300, contours=[]),                                                                          # 1 space
    dict(adv=1000, contours=[[(100, 0, 1), (400, 700, 1), (600, 700, 1), (900, 0, 1)]]),                 # 2 A
    dict(adv=1000, comps=[(2, 0, 0, None, 0x200), (5, 300, 800, None, 0)], bbox=(100, 0, 900, 900)),      # 3 Adieresis
    dict(adv=500, comps=[(2, 200, 100, 0.5, 0x800)], bbox=(150, 50, 550, 400)),                          # 4 small A
    dict(adv=0, contours=[[(0, 0, 1), (0, 100, 1), (100, 100, 1), (100, 0, 1)]]),                        # 5 dot
]
P_CMAP = {0x20: 1, 0x41: 2, 0xC4: 3, 0x61: 4, 0x2E: 5}
for _i, _c in enumerate(LETTERS):
    P_CMAP.setdefault(_c, (2, 4)[_i % 2])
P_GVARS = [
    None,
    dict(tuples=[dict(peak=[1, 0], dx=[0, 120, 0, 0], dy=[0, 0, 0, 0]), dict(peak=[-1, 0], dx=[0, -60, 0, 0], dy=[0] * 4)]),
    dict(shared_points=[0, 1, 2, 3, 4, 5], words=True, tuples=[                                          # pp1 + pp2
        dict(peak=[1, 0], dx=[-200, -40, 40, 200, -20, 360], dy=[0, 0, 0, 0, 0, 0], force="w"),
        dict(peak=[-1, 0], points=None, dx=[0, 0, 0, 0, 0, -50, 0, 0], dy=[0] * 8, force="l"),
        dict(peak=[0, 1], points=[1, 2], dx=[0, 0], dy=[100, 100]),                                     # opsz: taller
    ]),
    dict(tuples=[dict(peak=[1, 0], points=[1, 3], dx=[30, 100], dy=[-20, 0])]),                          # comp 1, pp2
    dict(tuples=[dict(peak=[1, 0], dx=[40, 0, 0, 0, 0], dy=[20, 0, 0, 0, 0])]),                          # scaled offset
    None,
]


def budget_font():
    n = 8000
    c = [(5 * (i // 2), 100 * (i % 2), 1) for i in range(n // 2)] + [(5 * (n // 2 - 1 - i // 2), 200 + 100 * (i % 2), 1)
                                                                    for i in range(n // 2)]
    glyphs = [dict(adv=500, contours=[[(0, 0, 1), (0, 500, 1), (500, 500, 1), (500, 0, 1)]]),
              dict(adv=20000, contours=[c])]
    tuples = [dict(peak=[1], points=[i % n], words=True, dx=[1], dy=[1]) for i in range(4095)]  # noqa: E501
    gv = [dict(tuples=[dict(peak=[1], dx=[10, 10, 10, 10, 0, 0, 0, 0], dy=[0] * 8)]), dict(tuples=tuples)]
    names = [(1, 0x409, "jfgui Var Budget"), (2, 0x409, "Regular")]
    return build(glyphs, {0x42: 1}, names, [("wght", 100, 400, 900, 0, 256)],
                 {"gvar": gvar_table(1, [], gv, True)})


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    store = ivs(4, REGIONS, [(HV_ROWS, 1, False)])
    mstore = ivs(4, REGIONS, [(MV_ROWS, 1, True)])
    extra = {"avar": avar_table(AVAR), "gvar": gvar_table(4, SHARED, GVARS, False), "HVAR": hvar_table(store, HV_MAP),
             "MVAR": mvar_table(MV_RECS, mstore), "instances": INSTANCES}
    (OUT / "jfvar.ttf").write_bytes(build(GLYPHS, CMAP, NAMES, AXES, extra))
    pextra = {"gvar": gvar_table(2, [], P_GVARS, True), "instances": P_INSTANCES}
    (OUT / "jfvar-phantom.ttf").write_bytes(build(P_GLYPHS, P_CMAP, P_NAMES, P_AXES, pextra))
    (OUT / "budget.bin").write_bytes(budget_font())

    exp = ""
    cases = [("jfvar.ttf", GLYPHS, GVARS, SHARED, AXES, AVAR, s) for s in
             ({"wght": 900}, {"wght": 650}, {"wght": 100, "wdth": 75}, {"wdth": 87.5, "slnt": -6, "TEST": 10},
              {"wght": 250, "wdth": 50, "slnt": -12})]
    cases += [("jfvar-phantom.ttf", P_GLYPHS, P_GVARS, [], P_AXES, None, s) for s in ({"wght": 800}, {"wght": 300},
                                                                                     {"wght": 600, "opsz": 30})]
    for path, glyphs, gvars, shared, axes, avar, user in cases:
        coords = normalize(axes, avar, user)
        spec = ",".join(f"{k}={v:g}" for k, v in user.items())
        exp += f"var {path} {spec} coords {' '.join(str(c) for c in coords)}\n"
        for gid, g in enumerate(glyphs):
            cont, pp = outline(glyphs, gvars, shared, coords, gid)
            if path == "jfvar.ttf":
                d = ivs_delta(REGIONS, HV_ROWS[HV_MAP[gid][1]], coords)
                adv = g["adv"] + math.floor(d + .5)
            else:
                # the metrics glyph: USE_MY_METRICS takes the component's hmtx advance
                mg = next((c[0] for c in g.get("comps", []) if c[4] & 0x200), gid)
                adv = glyphs[mg]["adv"] + math.floor(pp[1] - pp[0] + .5)
            shift = -pp[0]
            cont = [[(x + shift, y, on) for x, y, on in c] for c in cont if c]
            exp += fmt_outline(f"g{gid}", gid, adv, cont)
        if path == "jfvar.ttf":
            mv = {t: math.floor(ivs_delta(REGIONS, MV_ROWS[i], coords) + .5) for t, _, i in MV_RECS}
            exp += f"metrics asc {850 + mv['hasc']} typo {800 + mv['hasc']} xh {500 + mv['xhgt']} ul {50 + mv['unds']}\n"
    (OUT / "expected.txt").write_text(exp, encoding="utf-8", newline="\n")
    print("wrote", OUT)


if __name__ == "__main__":
    main()
