"""mkicons.py -- JFDiagrammer's ribbon icons (own artwork, NetLab look) -> src/icons_data.rs

    python apps/JFDiagrammer/mkicons.py                   the Rust data from icons/*.svg
    python apps/JFDiagrammer/mkicons.py --overview FILE   ... and all icons in a grid (PNG)
    python apps/JFDiagrammer/mkicons.py --sheet FILE      ... and the approved sample sheet (PNG)
      --part P[,P..] --prio P1[,P2]   only these rows (icons/icons.txt) for --overview / --zoom
      --zoom FILE                     the overview with every pixel 8x (checking the grid)
      --no-rust                       leave src/icons_data.rs alone (drawing in parallel)
      --strict                        a missing SVG is an error (else a note; the icon is left out)

icons/icons.txt lists every icon: its number (code point 0x10F200 + n for 16 px, 0x10F600 + n
for 32 px), its file stem, its sizes, priority, name, place and a motif hint.  Each size is
drawn by hand on its own pixel grid: icons/STEM_16.svg (viewBox 0 0 16 16, small buttons) and
icons/STEM_32.svg (0 0 32 32, large buttons).  Rules (FEATURES.md 7, JD14, approved 2026-11-16):

  grid        whole pixels: 1 px lines centred on pixel centres (x.5), fills on pixel edges;
              a 1 px margin at 16, 2 px at 32; diagonals at 45 degrees where it can be
  lines       1 px in both sizes, like the 1 DIP frames of the NetLab style; square corners
              (miter joins, butt caps); marks (plus, cross, letters) are 1 px small, 2 px large
  shapes      straight and square: a circle only where the thing is round (a lens, a clock, a
              globe, the ellipse tool); a container that is a circle elsewhere becomes a square
  colours     four roles, by the first / second class name of an element (vsym's role names):
                ink    lines and letters: the icon colour the style asks for (text, disabled)
                body   a neutral surface: the ink at 12 %
                tint   the accent (Signalblau) for the one element the command makes or moves
                hi     an accent surface: the accent at 24 % (like NetLab's selection tint)
              shade (the ink at 26 %) is there for a second neutral surface.  The colours in the
              SVG files are the light theme's, only for viewing; the palette decides.
  accent      at most one accent element per icon; a disabled icon is all ink
  overlaps    surfaces are translucent: a shape behind another is drawn only where it shows,
              with a 1 px gap to the front shape's outline

Strokes become filled outlines here (the library's vector icons only know round joins), so the
corners stay square at every size.  Each element is one or two layers of ui::Symbol (lib/ui/
symbol.rs): its fill, then its stroke.  The check warns about axis-parallel edges off the pixel
grid.  icons_data.rs is deterministic (a rerun writes the same bytes); do not edit it.

Needs a JFLib checkout for tools/vsym.py (SVG parsing) and, for the sheets, tools/refrender.py,
tools/ttf2bin.py and the Inter fonts of styles/netlab: JFLIB, else the repository around this
folder, else ../JFLib beside it.
"""
import argparse
import math
import os
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent


def jflib_root():
    for cand in (os.environ.get("JFLIB"), HERE.parent.parent, HERE.parent / "JFLib"):
        if cand and (Path(cand) / "tools" / "vsym.py").is_file():
            return Path(cand).resolve()
    sys.exit("mkicons: no JFLib checkout (set JFLIB)")


ROOT = jflib_root()
sys.path.insert(0, str(ROOT / "tools"))
import vsym  # noqa: E402

BASE = 0x10_F200  # the code point of icon n at 16 px
LARGE = 0x400  # ... + this at 32 px
SAMPLE = 0x010  # numbers below: the approved sample sheet
SIZES = (16, 32)
MARGIN = {16: 1.0, 32: 2.0}
MITER_LIMIT = 4.0  # SVG's default
FLAT = 0.08  # flattening tolerance in px (curves become polygons; small for 2x / 3x screens)


class IconError(Exception):
    pass


# ---------------------------------------------------------------- geometry
def flatten(subs, m):
    """vsym subpaths -> [(points, closed)] in grid units, curves as polylines"""
    out = []
    for start, segs, closed in subs:
        pts = [vsym.apply(m, *start)]
        for s in segs:
            if s[0] == "L":
                pts.append(vsym.apply(m, *s[1]))
                continue
            p0 = pts[-1]
            c1, c2, p3 = (vsym.apply(m, *q) for q in s[1:])
            dd = max(math.hypot(p0[0] - 2 * c1[0] + c2[0], p0[1] - 2 * c1[1] + c2[1]),
                     math.hypot(c1[0] - 2 * c2[0] + p3[0], c1[1] - 2 * c2[1] + p3[1]))
            n = max(2, min(64, math.ceil(math.sqrt(0.75 * dd / FLAT))))
            for i in range(1, n + 1):
                t = i / n
                u = 1 - t
                pts.append((u * u * u * p0[0] + 3 * u * u * t * c1[0] + 3 * u * t * t * c2[0] + t * t * t * p3[0],
                            u * u * u * p0[1] + 3 * u * u * t * c1[1] + 3 * u * t * t * c2[1] + t * t * t * p3[1]))
        clean = [pts[0]]
        for p in pts[1:]:
            if math.hypot(p[0] - clean[-1][0], p[1] - clean[-1][1]) > 1e-9:
                clean.append(p)
        if closed and len(clean) > 2 and math.hypot(clean[0][0] - clean[-1][0], clean[0][1] - clean[-1][1]) < 1e-9:
            clean.pop()
        out.append((clean, closed))
    return out


def area(poly):
    return 0.5 * sum(poly[i][0] * poly[(i + 1) % len(poly)][1] - poly[(i + 1) % len(poly)][0] * poly[i][1]
                     for i in range(len(poly)))


def oriented(poly, positive):
    return poly if (area(poly) > 0) == positive else poly[::-1]


def normal(p, q):
    """the unit normal to the left of p -> q (y down: left = (dy, -dx))"""
    dx, dy = q[0] - p[0], q[1] - p[1]
    n = math.hypot(dx, dy)
    return dy / n, -dx / n


def join(p, n0, n1, d):
    """the offset corner at p between two edges with normals n0, n1 at distance d: the miter
    point, or both offset ends past the miter limit (a bevel)"""
    a = (p[0] + n0[0] * d, p[1] + n0[1] * d)
    b = (p[0] + n1[0] * d, p[1] + n1[1] * d)
    cos = n0[0] * n1[0] + n0[1] * n1[1]
    if cos > 1 - 1e-12:
        return [a]
    if cos < -1 + 1e-9:
        return [a, b]
    # the miter point lies along n0 + n1 at d / cos(half angle)
    hx, hy = n0[0] + n1[0], n0[1] + n1[1]
    hn = math.hypot(hx, hy)
    k = math.sqrt((1 + cos) / 2)  # cos of half the turn
    if 1 / k > MITER_LIMIT:
        return [a, b]
    return [(p[0] + hx / hn * d / k, p[1] + hy / hn * d / k)]


def offset_closed(pts, d):
    n = len(pts)
    norms = [normal(pts[i], pts[(i + 1) % n]) for i in range(n)]
    out = []
    for i in range(n):
        out += join(pts[i], norms[i - 1], norms[i], d)
    return out


def offset_open(pts, d):
    norms = [normal(pts[i], pts[i + 1]) for i in range(len(pts) - 1)]
    out = [(pts[0][0] + norms[0][0] * d, pts[0][1] + norms[0][1] * d)]
    for i in range(1, len(pts) - 1):
        out += join(pts[i], norms[i - 1], norms[i], d)
    out.append((pts[-1][0] + norms[-1][0] * d, pts[-1][1] + norms[-1][1] * d))
    return out


def stroke_polys(paths, w, cap):
    """stroked subpaths -> polygons for the non-zero rule: rings as an outer loop (positive) and
    an inner loop (negative), open lines as one positive loop each (overlaps join)"""
    h = w / 2
    polys = []
    for pts, closed in paths:
        if len(pts) < 2:
            continue
        if closed and len(pts) >= 3:
            a, b = offset_closed(pts, h), offset_closed(pts, -h)
            outer, inner = (a, b) if abs(area(a)) > abs(area(b)) else (b, a)
            polys.append(oriented(outer, True))
            polys.append(oriented(inner, False))
            continue
        if cap == "square":
            (x0, y0), (x1, y1) = pts[0], pts[1]
            l0 = math.hypot(x1 - x0, y1 - y0)
            (xa, ya), (xb, yb) = pts[-2], pts[-1]
            l1 = math.hypot(xb - xa, yb - ya)
            pts = [(x0 - (x1 - x0) / l0 * h, y0 - (y1 - y0) / l0 * h)] + pts[1:-1] + \
                  [(xb + (xb - xa) / l1 * h, yb + (yb - ya) / l1 * h)]
        left, right = offset_open(pts, h), offset_open(pts, -h)
        polys.append(oriented(left + right[::-1], True))
    return polys


def fmt(v):
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "", "-") else s


def path_data(polys):
    out = []
    for poly in polys:
        out.append("M" + fmt(poly[0][0]) + " " + fmt(poly[0][1]))
        cur = poly[0]
        for p in poly[1:]:
            x, y = fmt(p[0]), fmt(p[1])
            if y == fmt(cur[1]) and x != fmt(cur[0]):
                out.append("H" + x)
            elif x == fmt(cur[0]) and y != fmt(cur[1]):
                out.append("V" + y)
            else:
                out.append("L" + x + " " + y)
            cur = p
        out.append("Z")
    return "".join(out)


# ---------------------------------------------------------------- SVG -> layers
def walk(el, st, layers, warns, where):
    tag = el.tag.removeprefix(vsym.SVG_NS)
    if el.tag.startswith("{") and not el.tag.startswith(vsym.SVG_NS):
        raise IconError(f"{where}: element {el.tag} outside the SVG namespace")
    if tag in vsym.SKIP:
        return
    if tag not in vsym.SHAPES and tag not in ("g", "svg"):
        raise IconError(f"{where}: unknown element <{tag}>")
    st = dict(st)
    for k in ("class", "fill", "stroke", "stroke-width", "fill-rule", "stroke-linecap", "stroke-linejoin"):
        if el.get(k) is not None:
            st[k] = el.get(k).strip()
    if el.get("transform"):
        st["m"] = vsym.mul(st["m"], vsym.parse_transform(el.get("transform"), where))
    here = f"{where} <{tag}{' id=' + el.get('id') if el.get('id') else ''}>"
    if tag in ("g", "svg"):
        for i, ch in enumerate(el):
            walk(ch, st, layers, warns, f"{where}/{tag}[{i}]")
        return
    if "class" not in st:
        raise IconError(f"{here}: no class (role)")
    try:
        roles = vsym.roles_of(st["class"], here)
    except vsym.SymError as e:
        raise IconError(str(e)) from None
    if st.get("stroke-linejoin", "miter") != "miter":
        raise IconError(f"{here}: only miter joins (square corners)")
    paths = flatten(vsym.shape_subs(el, tag, here), st["m"])
    filled = st.get("fill", "black") != "none" and tag not in ("line", "polyline")
    stroked = st.get("stroke", "none") != "none"
    if not filled and not stroked:
        warns.append(f"{here}: neither filled nor stroked")
    if filled:
        polys = [p for p, _ in paths if len(p) >= 3]
        rule = "E" if st.get("fill-rule") == "evenodd" else "F"
        # a fill under its own stroke may follow the stroke's centre line (off the grid)
        layers.append((roles[0], rule, polys, here, not stroked))
    if stroked:
        role = roles[1] if len(roles) == 2 else (vsym.ROLES.index("ink") if filled else roles[0])
        a, b, c, d, _, _ = st["m"]
        w = float(st.get("stroke-width", "1").removesuffix("px")) * math.sqrt(abs(a * d - b * c))
        layers.append((role, "F", stroke_polys(paths, w, st.get("stroke-linecap", "butt")), here, True))


def check(layers, size, warns, where):
    lo, hi = MARGIN[size], size - MARGIN[size]
    for _, _, polys, here, grid in layers:
        bad = out = None
        for poly in polys:
            for i, p in enumerate(poly):
                q = poly[(i + 1) % len(poly)]
                if not (lo - 0.01 <= p[0] <= hi + 0.01 and lo - 0.01 <= p[1] <= hi + 0.01):
                    out = out or p
                for k in (0, 1) if grid else ():  # an edge along an axis must sit on a pixel edge
                    if abs(p[k] - q[k]) < 1e-9 and abs(p[1 - k] - q[1 - k]) > 1e-9 and abs(p[k] - round(p[k])) > 1e-6:
                        bad = bad or p
        if out:
            warns.append(f"{here}: ({fmt(out[0])}, {fmt(out[1])}) inside the {fmt(MARGIN[size])} px margin")
        if bad:
            warns.append(f"{here}: an axis-parallel edge off the pixel grid near ({fmt(bad[0])}, {fmt(bad[1])})")


def icon(path, size):
    """one SVG file -> ([(role, rule, polygons)], art, warnings)"""
    where = path.name
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as e:
        raise IconError(f"{where}: {e}") from None
    if root.tag != vsym.SVG_NS + "svg":
        raise IconError(f"{where}: the root is not an <svg> in the SVG namespace")
    vb = [float(v) for v in (root.get("viewBox") or "").replace(",", " ").split()]
    if vb != [0.0, 0.0, float(size), float(size)]:
        raise IconError(f"{where}: viewBox must be \"0 0 {size} {size}\"")
    layers, warns = [], []
    walk(root, {"m": vsym.IDENT}, layers, warns, where)
    if not layers:
        raise IconError(f"{where}: nothing painted")
    check(layers, size, warns, where)
    art = "|".join(f"#{role}{rule}{path_data(polys)}" for role, rule, polys, *_ in layers if polys)
    return [(r, ru, p) for r, ru, p, *_ in layers], art, warns


def table():
    """icons/icons.txt -> rows {part, n, cp, stem, const, sizes, prio, name, place, motif}"""
    rows, part, seen = [], None, {}
    for k, line in enumerate((HERE / "icons" / "icons.txt").read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            part = line[1:-1]
            continue
        f = [c.strip() for c in line.split("|")]
        if len(f) != 7 or part is None:
            sys.exit(f"mkicons: icons.txt:{k}: a row is n | stem | sizes | prio | Name | Ort | Motiv, in a [part]")
        n, stem, sizes, prio, name, place, motif = f
        sizes = tuple(int(v) for v in sizes.split())
        if not sizes or any(v not in SIZES for v in sizes) or prio not in ("P1", "P2", "P3"):
            sys.exit(f"mkicons: icons.txt:{k}: sizes 16 / 32, prio P1..P3")
        n = int(n, 16)
        for key in (n, stem):
            if key in seen:
                sys.exit(f"mkicons: icons.txt:{k}: {key} also in line {seen[key]}")
            seen[key] = k
        rows.append({"part": part, "n": n, "cp": BASE + n, "stem": stem, "const": "JD_" + stem.upper(),
                     "sizes": sizes, "prio": prio, "name": name, "place": place, "motif": motif})
    return rows


def load(rows, strict):
    """the drawn sizes of each row: row[16] / row[32] = (layers, art); missing files are noted"""
    failed, missing = False, []
    for row in rows:
        for size in row["sizes"]:
            f = HERE / "icons" / f"{row['stem']}_{size}.svg"
            if not f.is_file():
                missing.append(f.name)
                continue
            try:
                layers, art, warns = icon(f, size)
            except (IconError, vsym.SymError) as e:
                print(f"mkicons: error: {e}", file=sys.stderr)
                failed = True
                continue
            for w in warns:
                print(f"mkicons: warning: {w}", file=sys.stderr)
            row[size] = (layers, art)
    extra = sorted({p.name for p in (HERE / "icons").glob("*.svg")}
                   - {f"{r['stem']}_{s}.svg" for r in table() for s in r["sizes"]})
    for name in extra:
        print(f"mkicons: warning: {name} is not in icons.txt", file=sys.stderr)
    if missing:
        print(f"mkicons: {'error' if strict else 'note'}: {len(missing)} SVG files not drawn yet"
              + (f" ({', '.join(missing[:6])}{', ...' if len(missing) > 6 else ''})"), file=sys.stderr)
        failed |= strict
    if failed:
        sys.exit(1)
    return rows


def write_rust(rows):
    out = HERE / "src" / "icons_data.rs"
    lines = [
        "// icons_data -- JFDiagrammer's ribbon icons (own artwork; see mkicons.py), from icons/*.svg and",
        "// icons/icons.txt; generated by mkicons.py (python apps/JFDiagrammer/mkicons.py), do not edit.",
        "// Each art is filled layers for ui::Symbol::parse(art, [grid, grid]) (lib/ui/symbol.rs);",
        "// roles: 0 ink, 1 body, 2 shade, 4 tint (the accent), 9 hi (the accent surface).",
        "",
    ]
    items = []
    for r in rows:
        for size, const, cp in ((16, r["const"], r["cp"]), (32, r["const"] + "_L", r["cp"] + LARGE)):
            if size not in r:
                continue
            lines.append(f"/// {r['name']} ({r['place']}), {size} px")
            lines.append(f"pub(crate) const {const}: u32 = 0x{cp >> 16:X}_{cp & 0xFFFF:04X};")
            items.append((cp, const, size, r[size][1]))
    lines += ["", "/// (code point, grid, art), sorted by code point",
              "pub(crate) static ARTS: &[(u32, f32, &str)] = &["]
    for _, const, grid, art in sorted(items):
        lines.append(f'    ({const}, {grid}.0, "{art}"),')
    lines.append("];")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    n = sum(a.count("|") + 1 for *_, a in items)
    print(f"{out.relative_to(HERE.parent.parent).as_posix() if HERE.parent.parent in out.parents else out}: "
          f"{len(items)} drawings of {len(rows)} icons, {n} layers")


# ---------------------------------------------------------------- the preview
# the NetLab tokens (styles/netlab/palette.rs, lib.rs): ribbon surface, text, disabled, accent
THEMES = {
    "hell": {"bg": 0xF6F7F8, "ink": 0x1D2228, "dis": 0xA0A7B0, "sub": 0x4D5560, "line": 0xD3D7DC,
             "accent": 0x1F5FA8, "page": 0xECEEF1},
    "dunkel": {"bg": 0x23272C, "ink": 0xE3E6EA, "dis": 0x5B636D, "sub": 0xAAB1BA, "line": 0x33393F,
               "accent": 0x6FA3E0, "page": 0x1B1E22},
}
ALPHA = {1: 0.12, 2: 0.26, 9: 0.24}  # body, shade, hi


def rgb(c):
    return ((c >> 16) & 255, (c >> 8) & 255, c & 255)


def palette(th, disabled=False):
    ink = th["dis"] if disabled else th["ink"]
    acc = ink if disabled else th["accent"]
    pal = {r: (rgb(ink), 1.0) for r in range(10)}
    pal[1] = (rgb(ink), ALPHA[1])
    pal[2] = (rgb(ink), ALPHA[2])
    pal[4] = (rgb(acc), 1.0)
    pal[9] = (rgb(acc), ALPHA[9])
    return pal


def paint_icon(cv, rr, np, layers, x, y, scale, pal):
    """the layers onto the canvas, the grid's (0, 0) at x, y px, scale px per grid unit"""
    for role, rule, polys in layers:
        segs = []
        for poly in polys:
            for i, p in enumerate(poly):
                q = poly[(i + 1) % len(poly)]
                segs.append((p[0] * scale, p[1] * scale, q[0] * scale, q[1] * scale))
        if not segs:
            continue
        w = h = int(math.ceil(max(max(s[0], s[2], s[1], s[3]) for s in segs))) + 2
        if rule == "E":  # even-odd: the parity of the coverage of each loop (exact on our grid)
            cov = np.zeros((h, w), np.float32)
            for poly in polys:
                s = np.array([(p[0] * scale, p[1] * scale, q[0] * scale, q[1] * scale)
                              for p, q in zip(poly, poly[1:] + poly[:1])], np.float64)
                c = rr.raster_area(s, w, h).astype(np.float32) / 255.0
                cov = np.abs(cov - c)
            m = np.rint(cov * 255).astype(np.uint8)
        else:
            m = rr.raster_area(np.array(segs, np.float64), w, h)
        color, alpha = pal[role]
        cv.mask(x, y, (m.astype(np.float32) * alpha).astype(np.uint8) if alpha < 1 else m, color)


def sgf_fonts(work):
    fonts = {}
    for key, ttf in (("reg", "Inter-Regular.ttf"), ("semi", "Inter-SemiBold.ttf")):
        out = Path(work) / f"inter_{key}.bin"
        if not out.is_file():
            subprocess.run([sys.executable, str(ROOT / "tools" / "ttf2bin.py"), "--font",
                            str(ROOT / "styles" / "netlab" / "assets" / ttf), "--out", str(out)],
                           check=True, stdout=subprocess.DEVNULL)
        fonts[key] = out
    return fonts


def magnified(cv, rr, np, layers, size, x, y, pal, bg, k):
    """the icon drawn at 1:1 onto bg, every pixel then k x k at x, y (the sheet's magnifier)"""
    tmp = rr.Canvas(size, size, bg)
    paint_icon(tmp, rr, np, layers, 0, 0, 1.0, pal)
    cv.px[y:y + size * k, x:x + size * k] = tmp.px.repeat(k, axis=0).repeat(k, axis=1)


def sheet(rows, path, zoom=1):
    """the preview: per icon its name and place, then 32 and 16 px, light and dark, each also
    disabled, and the light ones magnified 2x; zoom > 1 scales every pixel of the sheet up
    (nearest, with a faint pixel grid) for checking"""
    import numpy as np
    import refrender as rr
    with tempfile.TemporaryDirectory() as work:
        f = {k: rr.load_font(str(v)) for k, v in sgf_fonts(work).items()}
        col_name, cell, gap, row_h, lupe = 232, 92, 8, 80, 136
        cols = [("hell", False), ("hell", True), ("dunkel", False), ("dunkel", True)]
        width = col_name + len(cols) * (cell + gap) + lupe + 16
        head = 132
        height = head + len(rows) * row_h + 44
        light = THEMES["hell"]
        cv = rr.Canvas(width, height, rgb(light["page"]))
        cv.text(f["semi"], "JFDiagrammer – Menüband-Icons, Probeblatt (JD14)", 18, 16, 32, rgb(light["ink"]))
        for k, line in enumerate((
                "Raster 32 px (2 px Rand) und 16 px (1 px Rand), je Größe eigens gezeichnet. Linien 1 px auf",
                "der Pixelmitte wie die NetLab-Rahmen, Ecken eckig; Zeichen (Plus, Kreuz, Buchstaben) groß 2 px.",
                "Flächen: Tinte 12 %. Akzent Signalblau nur für das eine Element, um das es geht; inaktiv alles grau.")):
            cv.text(f["reg"], line, 12, 16, 56 + k * 17, rgb(light["sub"]))
        # the column heads and their backgrounds
        x0 = col_name
        for i, (th, dis) in enumerate(cols):
            t = THEMES[th]
            x = x0 + i * (cell + gap)
            cv.rect(x, head - 24, cell, len(rows) * row_h + 24 + 8, rgb(t["bg"]))
            label = th + (", inaktiv" if dis else "")
            cv.text(f["reg"], label, 11, x + 10, head - 9, rgb(t["sub"]))
        xl = x0 + len(cols) * (cell + gap)
        cv.rect(xl, head - 24, lupe, len(rows) * row_h + 24 + 8, rgb(light["bg"]))
        cv.text(f["reg"], "hell, Lupe 2×", 11, xl + 10, head - 9, rgb(light["sub"]))
        for j, r in enumerate(rows):
            y = head + j * row_h
            cv.text(f["semi"], r["name"], 13, 16, y + 36, rgb(light["ink"]))
            cv.text(f["reg"], r["place"], 11, 16, y + 52, rgb(light["sub"]))
            for i, (th, dis) in enumerate(cols):
                t = THEMES[th]
                x = x0 + i * (cell + gap)
                pal = palette(t, dis)
                paint_icon(cv, rr, np, r[32][0], x + 14, y + 24, 1.0, pal)
                paint_icon(cv, rr, np, r[16][0], x + 60, y + 32, 1.0, pal)
            pal = palette(light)
            magnified(cv, rr, np, r[32][0], 32, xl + 12, y + 8, pal, rgb(light["bg"]), 2)
            magnified(cv, rr, np, r[16][0], 16, xl + 88, y + 24, pal, rgb(light["bg"]), 2)
            if j:
                cv.rect(16, y, col_name - 24, 1, rgb(light["line"]))
        cv.text(f["reg"], "Quellen: apps/JFDiagrammer/icons/*_16.svg, *_32.svg  ·  Skript: apps/JFDiagrammer/mkicons.py",
                11, 16, height - 16, rgb(light["sub"]))
        save(cv, np, rr, path, zoom)


def save(cv, np, rr, path, zoom):
    """the canvas as PNG; zoom > 1: every pixel zoom x zoom with a faint pixel grid"""
    img = np.clip(np.rint(cv.px), 0, 255).astype(np.uint8)
    if zoom > 1:
        img = img.repeat(zoom, axis=0).repeat(zoom, axis=1)
        for k in range(0, img.shape[0], zoom):
            img[k, :, :] = (img[k, :, :].astype(np.int32) * 7 // 8).astype(np.uint8)
        for k in range(0, img.shape[1], zoom):
            img[:, k, :] = (img[:, k, :].astype(np.int32) * 7 // 8).astype(np.uint8)
    rr.write_png(str(path), img)
    print(f"{path}: {img.shape[1]} x {img.shape[0]}")


def overview(rows, path, what, zoom=1):
    """every row in a grid: name and stem, then 32 and 16 px on the light and the dark ribbon
    (a size not drawn yet stays empty)"""
    import numpy as np
    import refrender as rr
    with tempfile.TemporaryDirectory() as work:
        f = {k: rr.load_font(str(v)) for k, v in sgf_fonts(work).items()}
        ncol, cw, ch, head = 4, 200, 86, 64
        nrow = (len(rows) + ncol - 1) // ncol
        width, height = 16 + ncol * cw, head + nrow * ch + 16
        light, dark = THEMES["hell"], THEMES["dunkel"]
        cv = rr.Canvas(width, height, rgb(light["page"]))
        drawn = sum(1 for r in rows if all(s in r for s in r["sizes"]))
        cv.text(f["semi"], f"JFDiagrammer – Menüband-Icons: {what}", 16, 16, 28, rgb(light["ink"]))
        cv.text(f["reg"], f"{len(rows)} Icons, {drawn} fertig gezeichnet; je Feld 32 und 16 px, hell und dunkel",
                11, 16, 46, rgb(light["sub"]))
        for j, r in enumerate(rows):
            x, y = 16 + (j % ncol) * cw, head + (j // ncol) * ch
            cv.text(f["semi"], r["name"][:30], 11, x, y + 12, rgb(light["ink"]))
            cv.text(f["reg"], f"{r['stem']} · {r['prio']}", 10, x, y + 25, rgb(light["sub"]))
            for k, t in enumerate((light, dark)):
                bx = x + k * 92
                cv.rect(bx, y + 32, 88, 46, rgb(t["bg"]))
                pal = palette(t)
                if 32 in r:
                    paint_icon(cv, rr, np, r[32][0], bx + 7, y + 39, 1.0, pal)
                if 16 in r:
                    paint_icon(cv, rr, np, r[16][0], bx + 58, y + 47, 1.0, pal)
        save(cv, np, rr, path, zoom)


def main():
    ap = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    ap.add_argument("--sheet", help="the approved sample sheet (PNG)")
    ap.add_argument("--overview", help="the selected icons in a grid (PNG)")
    ap.add_argument("--zoom", help="the overview with every pixel 8x (PNG)")
    ap.add_argument("--part", help="only these parts of icons.txt (comma separated)")
    ap.add_argument("--prio", help="only these priorities (comma separated, e.g. P1)")
    ap.add_argument("--no-rust", action="store_true", help="leave src/icons_data.rs alone")
    ap.add_argument("--strict", action="store_true", help="a missing SVG file is an error")
    a = ap.parse_args()
    rows = table()
    parts = {p for r in rows for p in [r["part"]]}
    sel = rows
    if a.part:
        want = set(a.part.split(","))
        if want - parts:
            sys.exit(f"mkicons: unknown part {', '.join(sorted(want - parts))} (parts: {', '.join(sorted(parts))})")
        sel = [r for r in sel if r["part"] in want]
    if a.prio:
        sel = [r for r in sel if r["prio"] in set(a.prio.split(","))]
    full = sel is rows
    load(rows if full or not a.no_rust else sel, a.strict)
    if not a.no_rust:
        write_rust(rows)
    what = ", ".join(x for x in (a.part, a.prio) if x) or "alle"
    if a.overview:
        overview(sel, a.overview, what)
    if a.zoom:
        overview(sel, a.zoom, what, zoom=8)
    if a.sheet:
        sample = [r for r in rows if r["n"] < SAMPLE]
        if any(16 not in r or 32 not in r for r in sample):
            sys.exit("mkicons: the sample sheet needs both sizes of its icons")
        sheet(sorted(sample, key=lambda r: r["n"]), a.sheet)


if __name__ == "__main__":
    main()
