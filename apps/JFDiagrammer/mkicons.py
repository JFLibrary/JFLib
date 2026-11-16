"""mkicons.py -- JFDiagrammer's ribbon icons (own artwork, NetLab look) -> src/icons_data.rs

    python apps/JFDiagrammer/mkicons.py                  the Rust data from icons/*.svg
    python apps/JFDiagrammer/mkicons.py --sheet FILE     ... and the preview sheet (PNG)
    python apps/JFDiagrammer/mkicons.py --zoom FILE      ... and an 8x pixel view (checking)

Every icon is drawn twice, by hand, on its own pixel grid: icons/NAME_16.svg (viewBox 0 0 16 16,
small buttons) and icons/NAME_32.svg (0 0 32 32, large buttons).  Rules (FEATURES.md 7, JD14):

  grid        whole pixels: 1 px lines centred on pixel centres (x.5), fills on pixel edges;
              a 1 px margin at 16, 2 px at 32; diagonals at 45 degrees where it can be
  lines       1 px in both sizes, like the 1 DIP frames of the NetLab style; square corners
              (miter joins, butt caps); marks (plus, cross, letters) are 1 px small, 2 px large
  colours     four roles, by the first / second class name of an element (vsym's role names):
                ink    lines and letters: the icon colour the style asks for (text, disabled)
                body   a neutral surface: the ink at 12 %
                tint   the accent (Signalblau) for the one element the command makes or moves
                hi     an accent surface: the accent at 24 % (like NetLab's selection tint)
              shade (the ink at 26 %) is there for a second neutral surface.  The colours in the
              SVG files are the light theme's, only for viewing; the palette decides.
  accent      at most one accent element per icon; a disabled icon is all ink

Strokes become filled outlines here (the library's vector icons only know round joins), so the
corners stay square at every size.  Each element is one or two layers of ui::Symbol (lib/ui/
symbol.rs): its fill, then its stroke.  The check warns about axis-parallel edges off the pixel
grid.  icons_data.rs is deterministic (a rerun writes the same bytes); do not edit it.

Needs a JFLib checkout for tools/vsym.py (SVG parsing) and, for the sheet, tools/refrender.py,
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

# (file stem, constant, code point of the small icon, German name, register / group); the large
# icon is the code point + 0x80 (constant + "_L").  Code points stay once given.
ICONS = [
    ("pointer", "JD_POINTER", 0x10_F200, "Zeigertool", "Start · Tools"),
    ("connector", "JD_CONNECTOR", 0x10_F201, "Verbinder", "Start · Tools"),
    ("text", "JD_TEXT", 0x10_F202, "Text", "Start · Tools"),
    ("rectangle", "JD_RECTANGLE", 0x10_F203, "Rechteck", "Start · Tools"),
    ("connection_point", "JD_CONNECTION_POINT", 0x10_F204, "Verbindungspunkt", "Start · Tools"),
    ("format_painter", "JD_FORMAT_PAINTER", 0x10_F205, "Format übertragen", "Start · Zwischenablage"),
    ("align", "JD_ALIGN", 0x10_F206, "Ausrichten", "Start · Anordnen"),
    ("bring_front", "JD_BRING_FRONT", 0x10_F207, "In den Vordergrund", "Start · Anordnen"),
    ("group", "JD_GROUP", 0x10_F208, "Gruppieren", "Start · Anordnen"),
    ("new_page", "JD_NEW_PAGE", 0x10_F209, "Neue Seite", "Einfügen · Seiten"),
    ("container", "JD_CONTAINER", 0x10_F20A, "Container", "Einfügen · Diagrammteile"),
    ("relayout", "JD_RELAYOUT", 0x10_F20B, "Seite neu layouten", "Entwurf · Layout"),
    ("task_panes", "JD_TASK_PANES", 0x10_F20C, "Aufgabenbereiche", "Ansicht · Fenster"),
]
LARGE = 0x80
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


def load_all():
    rows, failed = [], False
    for stem, const, cp, name, place in ICONS:
        row = {"stem": stem, "const": const, "cp": cp, "name": name, "place": place}
        for size in SIZES:
            f = HERE / "icons" / f"{stem}_{size}.svg"
            if not f.is_file():
                print(f"mkicons: error: {f.name} missing", file=sys.stderr)
                failed = True
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
        rows.append(row)
    if failed:
        sys.exit(1)
    return rows


def write_rust(rows):
    out = HERE / "src" / "icons_data.rs"
    lines = [
        "// icons_data -- JFDiagrammer's ribbon icons (own artwork; see mkicons.py), from icons/*.svg;",
        "// generated by mkicons.py (python apps/JFDiagrammer/mkicons.py), do not edit.",
        "// Each art is filled layers for ui::Symbol::parse(art, [grid, grid]) (lib/ui/symbol.rs);",
        "// roles: 0 ink, 1 body, 2 shade, 4 tint (the accent), 9 hi (the accent surface).",
        "",
    ]
    for r in rows:
        lines.append(f"/// {r['name']} ({r['place']}), 16 px")
        lines.append(f"pub(crate) const {r['const']}: u32 = 0x{r['cp'] >> 16:X}_{r['cp'] & 0xFFFF:04X};")
        cpl = r["cp"] + LARGE
        lines.append(f"/// {r['name']}, 32 px (large buttons)")
        lines.append(f"pub(crate) const {r['const']}_L: u32 = 0x{cpl >> 16:X}_{cpl & 0xFFFF:04X};")
    lines += ["", "/// (code point, grid, art), sorted by code point",
              "pub(crate) static ARTS: &[(u32, f32, &str)] = &["]
    items = []
    for r in rows:
        items.append((r["cp"], r["const"], 16, r[16][1]))
        items.append((r["cp"] + LARGE, r["const"] + "_L", 32, r[32][1]))
    for _, const, grid, art in sorted(items):
        lines.append(f'    ({const}, {grid}.0, "{art}"),')
    lines.append("];")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    n = sum(a.count("|") + 1 for *_, a in items)
    print(f"{out.relative_to(HERE.parent.parent).as_posix() if HERE.parent.parent in out.parents else out}: "
          f"{len(items)} icons, {n} layers")


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
        img = np.clip(np.rint(cv.px), 0, 255).astype(np.uint8)
        if zoom > 1:
            img = img.repeat(zoom, axis=0).repeat(zoom, axis=1)
            for k in range(0, img.shape[0], zoom):  # a faint pixel grid
                img[k, :, :] = (img[k, :, :].astype(np.int32) * 7 // 8).astype(np.uint8)
            for k in range(0, img.shape[1], zoom):
                img[:, k, :] = (img[:, k, :].astype(np.int32) * 7 // 8).astype(np.uint8)
        rr.write_png(str(path), img)
        print(f"{path}: {img.shape[1]} x {img.shape[0]}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    ap.add_argument("--sheet", help="write the preview sheet (PNG)")
    ap.add_argument("--zoom", help="write the sheet with every pixel 8x (PNG)")
    a = ap.parse_args()
    rows = load_all()
    write_rust(rows)
    if a.sheet:
        sheet(rows, a.sheet)
    if a.zoom:
        sheet(rows, a.zoom, zoom=8)


if __name__ == "__main__":
    main()
