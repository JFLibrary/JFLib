"""vsym.py -- multi-colour vector symbols for ui::Symbol (lib/ui/symbol.rs) from SVG files.

    python tools/vsym.py apps/JFNetworker

reads DIR/icons/*.svg (viewBox "0 0 64 48") and writes DIR/src/workspace/icons/symbols_data.rs:

    pub(crate) const SYMBOL_GRID: [f32; 2] = [64.0, 48.0];
    pub(crate) static SYMBOLS: &[(&str, &str)] = &[("pc", "<layers>"), ...];   (by file name)

Each painted element becomes one or two layers in the syntax of symbol.rs ('|' between
layers, prefix "#n" role, "F" / "E" fill, "S<w>" stroke width, then absolute path data with
M L C Z only): the fill first, then the stroke.

The SVG files are drawn by hand and stay readable in any SVG viewer:
  elements    svg, g, path, rect (rx, ry), circle, ellipse, line, polyline, polygon; title,
              desc, metadata and style are skipped; any other element is an error
  transform   translate, scale, rotate, skewX, skewY, matrix on any element -- baked into the
              coordinates (arcs become cubics first, so every affine transform is exact)
  class       "FILL [STROKE]": palette roles ink body shade screen tint port ok warn crit hi
              (0..9 in that order)
  painting    an element is filled when its fill is not "none" (SVG's default fill is black,
              so outlines need fill="none"), in the role of its first class; it is stroked when
              its stroke is set and not "none", in the role of its second class (default ink,
              or the first class when the element is not filled); stroke-width in grid units
              (SVG's default 1); fill-rule="evenodd" fills with the even-odd rule.  The colour
              values are ignored: the palette (SymPalette) decides.
  inherited   from g / svg: class (unlike CSS), fill, stroke, stroke-width, fill-rule
Geometry outside the 2-unit margin (2..62 x 2..46) is reported as a warning.  The output has
LF line endings and is deterministic (a rerun writes the same bytes).
"""
import math
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

SVG_NS = "{http://www.w3.org/2000/svg}"
ROLES = ["ink", "body", "shade", "screen", "tint", "port", "ok", "warn", "crit", "hi"]
GRID = (64, 48)
MARGIN = 2.0
STROKE_DEFAULT = 1.5  # symbol.rs's default (no "S" prefix needed)
SHAPES = {"path", "rect", "circle", "ellipse", "line", "polyline", "polygon"}
SKIP = {"title", "desc", "metadata", "style"}
KAPPA = 0.5522847498307936


class SymError(Exception):
    pass


# ---------------------------------------------------------------- numbers and matrices
def num(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "", "-") else s


IDENT = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)  # SVG order a b c d e f: x' = a x + c y + e


def mul(m, n):
    """m * n (n applied first)"""
    a, b, c, d, e, f = m
    a2, b2, c2, d2, e2, f2 = n
    return (a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2,
            a * e2 + c * f2 + e, b * e2 + d * f2 + f)


def apply(m, x, y):
    a, b, c, d, e, f = m
    return a * x + c * y + e, b * x + d * y + f


def parse_transform(s, where):
    m = IDENT
    pos = 0
    for t in re.finditer(r"\s*,?\s*(matrix|translate|scale|rotate|skewX|skewY)\s*\(([^)]*)\)", s):
        if t.start() != pos:
            raise SymError(f"{where}: bad transform {s!r}")
        pos = t.end()
        v = [float(x) for x in re.split(r"[\s,]+", t.group(2).strip()) if x]
        k = t.group(1)
        if k == "matrix" and len(v) == 6:
            n = tuple(v)
        elif k == "translate" and len(v) in (1, 2):
            n = (1, 0, 0, 1, v[0], v[1] if len(v) == 2 else 0)
        elif k == "scale" and len(v) in (1, 2):
            n = (v[0], 0, 0, v[1] if len(v) == 2 else v[0], 0, 0)
        elif k == "rotate" and len(v) in (1, 3):
            r = math.radians(v[0])
            n = (math.cos(r), math.sin(r), -math.sin(r), math.cos(r), 0, 0)
            if len(v) == 3:
                n = mul(mul((1, 0, 0, 1, v[1], v[2]), n), (1, 0, 0, 1, -v[1], -v[2]))
        elif k == "skewX" and len(v) == 1:
            n = (1, 0, math.tan(math.radians(v[0])), 1, 0, 0)
        elif k == "skewY" and len(v) == 1:
            n = (1, math.tan(math.radians(v[0])), 0, 1, 0, 0)
        else:
            raise SymError(f"{where}: bad transform {t.group(0).strip()!r}")
        m = mul(m, n)
    if s[pos:].strip():
        raise SymError(f"{where}: bad transform {s!r}")
    return m


# ---------------------------------------------------------------- path data
class Scan:
    """the SVG path mini-language: numbers, flags, command letters"""
    NUM = re.compile(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")

    def __init__(self, s, where):
        self.s, self.i, self.where = s, 0, where

    def blank(self):
        while self.i < len(self.s) and self.s[self.i] in " \t\r\n,":
            self.i += 1

    def done(self):
        self.blank()
        return self.i >= len(self.s)

    def cmd(self):
        self.blank()
        c = self.s[self.i] if self.i < len(self.s) else ""
        if c and c in "MmLlHhVvCcSsQqTtAaZz":
            self.i += 1
            return c
        return None

    def at_number(self):
        self.blank()
        return self.i < len(self.s) and (self.s[self.i].isdigit() or self.s[self.i] in "+-.")

    def number(self):
        self.blank()
        m = self.NUM.match(self.s, self.i)
        if not m:
            raise SymError(f"{self.where}: a number expected at {self.s[self.i:self.i + 12]!r}")
        self.i = m.end()
        return float(m.group(0))

    def flag(self):
        self.blank()
        c = self.s[self.i] if self.i < len(self.s) else ""
        if c not in ("0", "1"):
            raise SymError(f"{self.where}: an arc flag expected at {self.s[self.i:self.i + 12]!r}")
        self.i += 1
        return c == "1"


def arc_cubics(x1, y1, rx, ry, phi, large, sweep, x2, y2):
    """an SVG elliptical arc as cubics [(c1, c2, p)] (SVG 1.1 implementation notes F.6)"""
    if (x1, y1) == (x2, y2):
        return []
    rx, ry = abs(rx), abs(ry)
    if rx == 0 or ry == 0:
        return [((x1, y1), (x2, y2), (x2, y2))]
    p = math.radians(phi)
    cp, sp = math.cos(p), math.sin(p)
    dx, dy = (x1 - x2) / 2, (y1 - y2) / 2
    x1p, y1p = cp * dx + sp * dy, -sp * dx + cp * dy
    lam = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry)
    if lam > 1:
        rx, ry = rx * math.sqrt(lam), ry * math.sqrt(lam)
    num_ = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    co = math.sqrt(max(0.0, num_ / den)) if den else 0.0
    if large == sweep:
        co = -co
    cxp, cyp = co * rx * y1p / ry, -co * ry * x1p / rx
    cx = cp * cxp - sp * cyp + (x1 + x2) / 2
    cy = sp * cxp + cp * cyp + (y1 + y2) / 2

    def ang(ux, uy, vx, vy):
        a = math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        return a

    t1 = ang(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
    dt = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
    if not sweep and dt > 0:
        dt -= 2 * math.pi
    elif sweep and dt < 0:
        dt += 2 * math.pi
    n = max(1, math.ceil(abs(dt) / (math.pi / 2) - 1e-9))
    d = dt / n
    k = 4 / 3 * math.tan(d / 4)
    out = []

    def pt(t):
        ex, ey = rx * math.cos(t), ry * math.sin(t)
        return cp * ex - sp * ey + cx, sp * ex + cp * ey + cy

    def dv(t):
        ex, ey = -rx * math.sin(t), ry * math.cos(t)
        return cp * ex - sp * ey, sp * ex + cp * ey

    t = t1
    for i in range(n):
        a0, a1 = t, t + d
        p0, p3 = pt(a0), pt(a1)
        d0, d1 = dv(a0), dv(a1)
        c1 = (p0[0] + k * d0[0], p0[1] + k * d0[1])
        c2 = (p3[0] - k * d1[0], p3[1] - k * d1[1])
        if i == n - 1:
            p3 = (x2, y2)
        out.append((c1, c2, p3))
        t = a1
    return out


def parse_path(d, where):
    """SVG path data -> subpaths [(start, [("L", p) | ("C", c1, c2, p)], closed)] (absolute)"""
    sc = Scan(d, where)
    subs = []
    cur = None
    x = y = 0.0
    sx = sy = 0.0
    last = None  # previous command letter (upper case) for S / T
    lc2 = lq = None  # last cubic second control point, last quadratic control point
    c = None
    while not sc.done():
        k = sc.cmd()
        if k is None:
            if c is None or c in "Zz":
                raise SymError(f"{where}: a path command expected at {d[sc.i:sc.i + 12]!r}")
            k = c
            if k == "M":
                k = "L"
            elif k == "m":
                k = "l"
        c = k
        rel = k.islower()
        K = k.upper()
        ox, oy = (x, y) if rel else (0.0, 0.0)
        if K == "Z":
            if cur is not None:
                cur[2] = True
                x, y = sx, sy
            last = "Z"
            continue
        if K == "M":
            x, y = sc.number() + ox, sc.number() + oy
            sx, sy = x, y
            cur = [(x, y), [], False]
            subs.append(cur)
            last = "M"
            continue
        if cur is None or cur[2]:
            cur = [(x, y), [], False]
            subs.append(cur)
        if K == "L":
            x, y = sc.number() + ox, sc.number() + oy
            cur[1].append(("L", (x, y)))
        elif K == "H":
            x = sc.number() + ox
            cur[1].append(("L", (x, y)))
        elif K == "V":
            y = sc.number() + (y if rel else 0.0)
            cur[1].append(("L", (x, y)))
        elif K in "CS":
            if K == "C":
                c1 = (sc.number() + ox, sc.number() + oy)
            else:
                c1 = (2 * x - lc2[0], 2 * y - lc2[1]) if last in ("C", "S") else (x, y)
            c2 = (sc.number() + ox, sc.number() + oy)
            p = (sc.number() + ox, sc.number() + oy)
            cur[1].append(("C", c1, c2, p))
            lc2 = c2
            x, y = p
        elif K in "QT":
            if K == "Q":
                q = (sc.number() + ox, sc.number() + oy)
            else:
                q = (2 * x - lq[0], 2 * y - lq[1]) if last in ("Q", "T") else (x, y)
            p = (sc.number() + ox, sc.number() + oy)
            c1 = (x + 2 / 3 * (q[0] - x), y + 2 / 3 * (q[1] - y))
            c2 = (p[0] + 2 / 3 * (q[0] - p[0]), p[1] + 2 / 3 * (q[1] - p[1]))
            cur[1].append(("C", c1, c2, p))
            lq = q
            x, y = p
        elif K == "A":
            rx, ry, phi = sc.number(), sc.number(), sc.number()
            large, sweep = sc.flag(), sc.flag()
            p = (sc.number() + ox, sc.number() + oy)
            for c1, c2, q in arc_cubics(x, y, rx, ry, phi, large, sweep, p[0], p[1]):
                cur[1].append(("C", c1, c2, q))
            x, y = p
        last = K
    return subs


# ---------------------------------------------------------------- shapes
def attr_f(el, name, where, default=None):
    v = el.get(name)
    if v is None:
        if default is None:
            raise SymError(f"{where}: {name} missing")
        return default
    try:
        return float(v.strip().removesuffix("px"))
    except ValueError:
        raise SymError(f"{where}: {name}={v!r} is not a number") from None


def ellipse_sub(cx, cy, rx, ry):
    """a closed ellipse, clockwise on screen from its right end"""
    kx, ky = rx * KAPPA, ry * KAPPA
    segs = [("C", (cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry)),
            ("C", (cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy)),
            ("C", (cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry)),
            ("C", (cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy))]
    return [(cx + rx, cy), segs, True]


def shape_subs(el, tag, where):
    if tag == "path":
        return parse_path(el.get("d", ""), where)
    if tag == "rect":
        x, y = attr_f(el, "x", where, 0.0), attr_f(el, "y", where, 0.0)
        w, h = attr_f(el, "width", where), attr_f(el, "height", where)
        if w <= 0 or h <= 0:
            raise SymError(f"{where}: empty rect")
        rx, ry = el.get("rx"), el.get("ry")
        rx = attr_f(el, "rx", where) if rx is not None else None
        ry = attr_f(el, "ry", where) if ry is not None else None
        rx = ry if rx is None else rx
        ry = rx if ry is None else ry
        rx, ry = min(rx or 0.0, w / 2), min(ry or 0.0, h / 2)
        if rx <= 0 or ry <= 0:
            return [[(x, y), [("L", (x + w, y)), ("L", (x + w, y + h)), ("L", (x, y + h))], True]]
        kx, ky = rx * KAPPA, ry * KAPPA
        r, b = x + w, y + h
        segs = [("L", (r - rx, y)), ("C", (r - rx + kx, y), (r, y + ry - ky), (r, y + ry)),
                ("L", (r, b - ry)), ("C", (r, b - ry + ky), (r - rx + kx, b), (r - rx, b)),
                ("L", (x + rx, b)), ("C", (x + rx - kx, b), (x, b - ry + ky), (x, b - ry)),
                ("L", (x, y + ry)), ("C", (x, y + ry - ky), (x + rx - kx, y), (x + rx, y))]
        return [[(x + rx, y), segs, True]]
    if tag == "circle":
        r = attr_f(el, "r", where)
        return [ellipse_sub(attr_f(el, "cx", where, 0.0), attr_f(el, "cy", where, 0.0), r, r)]
    if tag == "ellipse":
        return [ellipse_sub(attr_f(el, "cx", where, 0.0), attr_f(el, "cy", where, 0.0),
                            attr_f(el, "rx", where), attr_f(el, "ry", where))]
    if tag == "line":
        p0 = (attr_f(el, "x1", where, 0.0), attr_f(el, "y1", where, 0.0))
        p1 = (attr_f(el, "x2", where, 0.0), attr_f(el, "y2", where, 0.0))
        return [[p0, [("L", p1)], False]]
    if tag in ("polyline", "polygon"):
        v = [float(t) for t in re.split(r"[\s,]+", el.get("points", "").strip()) if t]
        if len(v) < 4 or len(v) % 2:
            raise SymError(f"{where}: bad points")
        pts = list(zip(v[0::2], v[1::2]))
        return [[pts[0], [("L", p) for p in pts[1:]], tag == "polygon"]]
    raise SymError(f"{where}: unknown element <{tag}>")


def emit(subs, m):
    """absolute path data; lines along an axis (as written) become H / V"""
    out = []
    for start, segs, closed in subs:
        cur = tuple(num(v) for v in apply(m, *start))
        out.append("M" + " ".join(cur))
        for s in segs:
            pts = [tuple(num(v) for v in apply(m, *p)) for p in s[1:]]
            if s[0] == "L" and pts[0][1] == cur[1] and pts[0][0] != cur[0]:
                out.append("H" + pts[0][0])
            elif s[0] == "L" and pts[0][0] == cur[0] and pts[0][1] != cur[1]:
                out.append("V" + pts[0][1])
            else:
                out.append(s[0] + " ".join(v for p in pts for v in p))
            cur = pts[-1]
        if closed:
            out.append("Z")
    return "".join(out)


def points(subs, m):
    for start, segs, _ in subs:
        yield apply(m, *start)
        for s in segs:
            yield apply(m, *s[-1])


# ---------------------------------------------------------------- the document
def roles_of(cls, where):
    names = cls.split()
    if not names or len(names) > 2:
        raise SymError(f"{where}: class must name one or two roles ({' '.join(ROLES)}), got {cls!r}")
    for n in names:
        if n not in ROLES:
            raise SymError(f"{where}: unknown role {n!r} (roles: {' '.join(ROLES)})")
    return [ROLES.index(n) for n in names]


def walk(el, state, layers, warns, where):
    tag = el.tag.removeprefix(SVG_NS)
    if el.tag.startswith("{") and not el.tag.startswith(SVG_NS):
        raise SymError(f"{where}: element {el.tag} outside the SVG namespace")
    if tag in SKIP:
        return
    if tag not in SHAPES and tag not in ("g", "svg"):
        raise SymError(f"{where}: unknown element <{tag}>")
    st = dict(state)
    for k in ("class", "fill", "stroke", "stroke-width", "fill-rule"):
        if el.get(k) is not None:
            st[k] = el.get(k).strip()
    if el.get("transform"):
        st["m"] = mul(st["m"], parse_transform(el.get("transform"), where))
    eid = el.get("id")
    here = f"{where} <{tag}{' id=' + eid if eid else ''}>"
    if tag in ("g", "svg"):
        for i, ch in enumerate(el):
            walk(ch, st, layers, warns, f"{where}/{tag}[{i}]")
        return
    if "class" not in st:
        raise SymError(f"{here}: no class (palette role)")
    roles = roles_of(st["class"], here)
    subs = shape_subs(el, tag, here)
    m = st["m"]
    for x, y in points(subs, m):
        if not (MARGIN - 0.01 <= x <= GRID[0] - MARGIN + 0.01 and MARGIN - 0.01 <= y <= GRID[1] - MARGIN + 0.01):
            warns.append(f"{here}: point ({num(x)}, {num(y)}) outside the 2-unit margin")
            break
    data = emit(subs, m)
    filled = st.get("fill", "black") != "none" and tag != "line"
    stroked = st.get("stroke", "none") != "none"
    if not filled and not stroked:
        warns.append(f"{here}: neither filled nor stroked")
    if filled:
        rule = "E" if st.get("fill-rule") == "evenodd" else "F"
        layers.append(f"#{roles[0]}{rule}{data}")
    if stroked:
        role = roles[1] if len(roles) == 2 else (ROLES.index("ink") if filled else roles[0])
        w = float(st.get("stroke-width", "1").removesuffix("px"))
        a, b, c, d, _, _ = m
        w *= math.sqrt(abs(a * d - b * c))
        sw = "" if abs(w - STROKE_DEFAULT) < 0.005 else f"S{num(w)}"
        layers.append(f"#{role}{sw}{data}")


def symbol(path):
    """one SVG file -> (art, warnings)"""
    where = path.name
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as e:
        raise SymError(f"{where}: {e}") from None
    if root.tag != SVG_NS + "svg":
        raise SymError(f"{where}: the root is not an <svg> in the SVG namespace")
    vb = [float(v) for v in re.split(r"[\s,]+", (root.get("viewBox") or "").strip()) if v]
    if vb != [0.0, 0.0, float(GRID[0]), float(GRID[1])]:
        raise SymError(f"{where}: viewBox must be \"0 0 {GRID[0]} {GRID[1]}\"")
    layers, warns = [], []
    walk(root, {"m": IDENT}, layers, warns, where)
    if not layers:
        raise SymError(f"{where}: nothing painted")
    art = "|".join(layers)
    if '"' in art or "\\" in art:
        raise SymError(f"{where}: unexpected characters in the art")
    return art, warns


def main(argv):
    if len(argv) != 1:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    d = Path(argv[0])
    files = sorted((d / "icons").glob("*.svg"), key=lambda p: p.name)
    if not files:
        print(f"vsym: no {d / 'icons'}/*.svg", file=sys.stderr)
        return 1
    rows, failed = [], False
    for f in files:
        try:
            art, warns = symbol(f)
        except SymError as e:
            print(f"vsym: error: {e}", file=sys.stderr)
            failed = True
            continue
        for w in warns:
            print(f"vsym: warning: {w}", file=sys.stderr)
        name = f.stem
        if not re.fullmatch(r"[a-z0-9_]+", name):
            print(f"vsym: error: {f.name}: names are lower case letters, digits and _", file=sys.stderr)
            failed = True
            continue
        rows.append((name, art, art.count("|") + 1))
    if failed:
        return 1
    out = d / "src" / "workspace" / "icons" / "symbols_data.rs"
    rel = d.as_posix().rstrip("/")
    lines = [
        f"// symbols_data -- the device symbols of {d.name} ({len(rows)}, {GRID[0]} x {GRID[1]} grid), from",
        f"// {rel}/icons/*.svg; generated by tools/vsym.py ({'python tools/vsym.py ' + rel}), do not edit.",
        "// Each art is layers for ui::Symbol::parse(art, SYMBOL_GRID) (see lib/ui/symbol.rs).",
        "",
        "/// the symbols' grid (SVG viewBox) in units",
        f"pub(crate) const SYMBOL_GRID: [f32; 2] = [{GRID[0]}.0, {GRID[1]}.0];",
        "",
        "/// (name = the SVG file's stem, art), sorted by name",
        "pub(crate) static SYMBOLS: &[(&str, &str)] = &[",
    ]
    for name, art, _ in rows:
        lines.append(f'    ("{name}", "{art}"),')
    lines.append("];")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{out.as_posix()}: {len(rows)} symbols, {sum(n for _, _, n in rows)} layers")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
