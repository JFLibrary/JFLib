#!/usr/bin/env python3
"""Compares a page drawn by the JFDiagrammer viewer with Visio's own SVG export of it, shape by shape.

Stage JD4 (apps/JFDiagrammer/PLAN.md) draws Visio pages; apps/diagramiotest writes a debug SVG of a
page from our display list. This tool reads that SVG ("ours") and the SVG Visio exported of the
same page ("ref") and compares every shape found in both: the outline (curves and arcs flattened to
chords within 0.002 pt) by bounding box and symmetric Hausdorff distance, and the text (content with
whitespace normalized, anchor of the first line). Python stdlib only.

  python -I apps/diagramiotest/svgcmp.py OURS.svg REF.svg [--tol 1.5] [--rel 0.01] [--text-tol 6]
         [--strict-text] [--json OUT.json] [--max-report 30]
  python -I apps/diagramiotest/svgcmp.py --ours DIR --ref DIR [same options]
  python -I apps/diagramiotest/svgcmp.py --selftest

Reference files: apps/JFDiagrammer/context/reference/<file stem>.p<page number>.svg, exported from
Visio with File > Save As > SVG, one file per page; the page number is the position among the
foreground pages, 1-based ("<file stem>.svg" alone counts as page 1). That folder is local only and
never committed (context/ is gitignored, it holds Microsoft content). In folder mode a reference
pairs with ours "<file stem>.p<N>.svg", else with the file of ours whose page <title> is the same;
without reference files (or without the folder) the tool prints "no reference SVGs found", exit 0.

Both files: user units are points, y down, origin at the top left of the page. A shape is the
innermost <g> with v:mID (or data-mid); its key is the page group (v:groupContext foregroundPage /
backgroundPage with its <title>) plus the ID, so background IDs do not collide with foreground
ones. Geometry right in a group's <g> belongs to the group. Transforms are composed from the root
(ours may have none). Skipped: defs, marker, symbol, style, clipPath, pattern, mask, use, v:*
elements, shadow groups (<g id="shadow...">), elements with data-jf="arrow", display:none.

A shape passes when the largest bounding box edge difference and the Hausdorff distance are both
within tol + rel * (diagonal of the reference bounding box). Text differences (content, or the
first line's anchor farther apart than --text-tol) are reported and fail only with --strict-text.
Shapes without geometry and text are ignored; shapes found in one file only count as failures.
Exit code 0: all pass, 1: failures, 2: usage or parse errors.
"""

import argparse
import json
import math
import os
import re
import sys
import xml.etree.ElementTree as ET

SVG_NS = 'http://www.w3.org/2000/svg'
VISIO_NS = 'http://schemas.microsoft.com/visio/2003/SVGExtensions/'
XLINK_NS = 'http://www.w3.org/1999/xlink'
FLAT = 0.002          # largest chord deviation of flattened curves and arcs, pt
MAX_SAMPLES = 20000   # sample points per outline and direction
BRUTE = 20000         # samples * segments up to which the plain nearest search is used
SKIP = {'defs', 'marker', 'symbol', 'style', 'clipPath', 'pattern', 'mask', 'use', 'title', 'desc',
        'metadata', 'linearGradient', 'radialGradient', 'filter', 'script', 'foreignObject'}
GEOM = {'path', 'rect', 'image', 'ellipse', 'circle', 'line', 'polyline', 'polygon'}
CMDS = 'MmLlHhVvCcSsQqTtAaZz'
NUM = re.compile(r'[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?')
TRANSFORM_LIST = re.compile(r'\s*(?:[A-Za-z]+\s*\([^()]*\)\s*,?\s*)*')
IDENT = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
UNITS = {'': 1.0, 'px': 1.0, 'pt': 4 / 3, 'pc': 16.0, 'in': 96.0, 'cm': 96 / 2.54, 'mm': 9.6 / 2.54}
INF = float('inf')


class SvgError(Exception):
    pass


# ---------------------------------------------------------------- matrices and attribute values

def mul(m, n):
    a, b, c, d, e, f = m
    A, B, C, D, E, F = n
    return (a * A + c * B, b * A + d * B, a * C + c * D, b * C + d * D, a * E + c * F + e, b * E + d * F + f)


def apply(m, x, y):
    return (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])


def norm(m):
    """Largest singular value of the linear part."""
    a, b, c, d = m[:4]
    t = a * a + b * b + c * c + d * d
    det = a * d - b * c
    return math.sqrt(max(0.0, (t + math.sqrt(max(0.0, t * t - 4 * det * det))) / 2))


def parse_transform(s):
    """SVG transform list -> matrix, None when malformed."""
    if not TRANSFORM_LIST.fullmatch(s):
        return None
    m = IDENT
    for name, args in re.findall(r'([A-Za-z]+)\s*\(([^()]*)\)', s):
        v = [float(x) for x in NUM.findall(args)]
        name = name.lower()
        if name == 'matrix' and len(v) == 6:
            t = tuple(v)
        elif name == 'translate' and len(v) in (1, 2):
            t = (1.0, 0.0, 0.0, 1.0, v[0], v[1] if len(v) > 1 else 0.0)
        elif name == 'scale' and len(v) in (1, 2):
            t = (v[0], 0.0, 0.0, v[1] if len(v) > 1 else v[0], 0.0, 0.0)
        elif name == 'rotate' and len(v) in (1, 3):
            r = math.radians(v[0])
            t = (math.cos(r), math.sin(r), -math.sin(r), math.cos(r), 0.0, 0.0)
            if len(v) == 3:
                t = mul(mul((1.0, 0.0, 0.0, 1.0, v[1], v[2]), t), (1.0, 0.0, 0.0, 1.0, -v[1], -v[2]))
        elif name == 'skewx' and len(v) == 1:
            t = (1.0, 0.0, math.tan(math.radians(v[0])), 1.0, 0.0, 0.0)
        elif name == 'skewy' and len(v) == 1:
            t = (1.0, math.tan(math.radians(v[0])), 0.0, 1.0, 0.0, 0.0)
        else:
            return None
        m = mul(m, t)
    return m


def length(s, fs=16.0, default=0.0):
    """A length in user units (CSS units; em relative to fs)."""
    if s is None:
        return default
    m = re.match(r'\s*(' + NUM.pattern + r')\s*([A-Za-z%]*)', s)
    if not m:
        return default
    v, u = float(m.group(1)), m.group(2).lower()
    if u == 'em':
        return v * fs
    if u == 'ex':
        return v * fs / 2
    return v * UNITS[u] if u in UNITS else default


def first(s):
    """First value of a list attribute (x="1 2 3")."""
    if s is None or not s.strip():
        return None
    return re.split(r'[\s,]+', s.strip())[0]


def split_tag(tag):
    if not isinstance(tag, str):
        return None, ''
    if tag.startswith('{'):
        ns, _, name = tag[1:].partition('}')
        return ns, name
    return '', tag


def attr_local(el, name):
    """Attribute by local name in any namespace (v:mID, mID)."""
    for k, v in el.attrib.items():
        if k == name or k.endswith('}' + name):
            return v
    return None


def decls(s):
    out = {}
    for part in (s or '').split(';'):
        if ':' in part:
            k, v = part.split(':', 1)
            out[k.strip().lower()] = v.strip()
    return out


def parse_css(root):
    """Class rules (.st1 {...}) of all <style> elements."""
    rules = {}
    for el in root.iter():
        if split_tag(el.tag)[1] != 'style':
            continue
        txt = re.sub(r'/\*.*?\*/', '', ''.join(el.itertext()), flags=re.S)
        for sels, body in re.findall(r'([^{}]+)\{([^}]*)\}', txt):
            d = decls(body)
            for sel in sels.split(','):
                sel = sel.strip()
                if re.fullmatch(r'\.[\w-]+', sel):
                    rules.setdefault(sel[1:], {}).update(d)
    return rules


def prop(el, name, css):
    """Property from style attribute, class rules, presentation attribute (in that precedence)."""
    st = decls(el.get('style'))
    if name in st:
        return st[name]
    v = None
    for cls in (el.get('class') or '').split():
        v = css.get(cls, {}).get(name, v)
    return v if v is not None else el.get(name)


def hidden(el, css):
    return (prop(el, 'display', css) or '').strip().lower() == 'none'


def font_size(el, parent, css):
    v = prop(el, 'font-size', css)
    if not v:
        return parent
    v = v.strip().lower()
    if v.endswith('%'):
        try:
            return parent * float(v[:-1]) / 100
        except ValueError:
            return parent
    return length(v, parent, parent)


def child_title(el):
    for ch in el:
        ns, tag = split_tag(ch.tag)
        if tag == 'title' and ns in ('', SVG_NS):
            return ' '.join(''.join(ch.itertext()).split())
    return None


# ---------------------------------------------------------------- flattening

def arc_points(cx, cy, rx, ry, ph, t0, dt, ctm):
    """Points of an elliptical arc (axes rotated by ph radians) mapped by ctm, both ends included."""
    r = max(rx, ry) * norm(ctm)
    step = 2 * math.acos(1 - FLAT / r) if r > FLAT else math.pi / 2
    n = max(1, min(4096, int(math.ceil(abs(dt) / step))))
    co, si = math.cos(ph), math.sin(ph)
    out = []
    for i in range(n + 1):
        t = t0 + dt * i / n
        ex, ey = rx * math.cos(t), ry * math.sin(t)
        out.append(apply(ctm, cx + co * ex - si * ey, cy + si * ex + co * ey))
    return out


def cubic(out, ctm, x0, y0, x1, y1, x2, y2, x3, y3):
    (x0, y0), (x1, y1), (x2, y2), (x3, y3) = [apply(ctm, *p)
                                              for p in ((x0, y0), (x1, y1), (x2, y2), (x3, y3))]
    dd = max(math.hypot(x0 - 2 * x1 + x2, y0 - 2 * y1 + y2), math.hypot(x1 - 2 * x2 + x3, y1 - 2 * y2 + y3))
    n = max(1, min(4096, int(math.ceil(math.sqrt(0.75 * dd / FLAT)))))
    for i in range(1, n):
        t = i / n
        u = 1 - t
        a, b, c, d = u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t
        out.append((a * x0 + b * x1 + c * x2 + d * x3, a * y0 + b * y1 + c * y2 + d * y3))
    out.append((x3, y3))


def quad(out, ctm, x0, y0, x1, y1, x2, y2):
    (x0, y0), (x1, y1), (x2, y2) = [apply(ctm, *p) for p in ((x0, y0), (x1, y1), (x2, y2))]
    dd = math.hypot(x0 - 2 * x1 + x2, y0 - 2 * y1 + y2)
    n = max(1, min(4096, int(math.ceil(math.sqrt(dd / (4 * FLAT))))))
    for i in range(1, n):
        t = i / n
        u = 1 - t
        a, b, c = u * u, 2 * u * t, t * t
        out.append((a * x0 + b * x1 + c * x2, a * y0 + b * y1 + c * y2))
    out.append((x2, y2))


def arc(out, ctm, x1, y1, rx, ry, phi, fa, fs, x2, y2):
    """SVG endpoint arc -> center parameterization (SVG 1.1 F.6.5, radii corrected as in F.6.6)."""
    if x1 == x2 and y1 == y2:
        return
    rx, ry = abs(rx), abs(ry)
    if rx == 0 or ry == 0:
        out.append(apply(ctm, x2, y2))
        return
    ph = math.radians(phi % 360)
    co, si = math.cos(ph), math.sin(ph)
    hx, hy = (x1 - x2) / 2, (y1 - y2) / 2
    xp, yp = co * hx + si * hy, -si * hx + co * hy
    lam = (xp / rx) ** 2 + (yp / ry) ** 2
    if lam > 1:
        rx, ry = rx * math.sqrt(lam), ry * math.sqrt(lam)
    num = (rx * ry) ** 2 - (rx * yp) ** 2 - (ry * xp) ** 2
    den = (rx * yp) ** 2 + (ry * xp) ** 2
    k = math.sqrt(max(0.0, num / den))
    if fa == fs:
        k = -k
    cxp, cyp = k * rx * yp / ry, -k * ry * xp / rx
    cx = co * cxp - si * cyp + (x1 + x2) / 2
    cy = si * cxp + co * cyp + (y1 + y2) / 2
    t1 = math.atan2((yp - cyp) / ry, (xp - cxp) / rx)
    dt = math.atan2((-yp - cyp) / ry, (-xp - cxp) / rx) - t1
    if fs and dt < 0:
        dt += 2 * math.pi
    elif not fs and dt > 0:
        dt -= 2 * math.pi
    out.extend(arc_points(cx, cy, rx, ry, ph, t1, dt, ctm)[1:-1])
    out.append(apply(ctm, x2, y2))


class _Scan:
    def __init__(self, s):
        self.s, self.i, self.n = s, 0, len(s)

    def skip(self):
        s, i, n = self.s, self.i, self.n
        while i < n and s[i] in ' \t\r\n\f,':
            i += 1
        self.i = i
        return i < n

    def num(self):
        self.skip()
        m = NUM.match(self.s, self.i)
        if not m:
            raise ValueError('number expected at offset %d' % self.i)
        self.i = m.end()
        return float(m.group())

    def flag(self):
        self.skip()
        if self.i < self.n and self.s[self.i] in '01':
            self.i += 1
            return self.s[self.i - 1] == '1'
        raise ValueError('arc flag expected at offset %d' % self.i)


def flatten_path(d, ctm, warn=None):
    """Path data -> polylines in page coordinates. Stops at the first error, as SVG renders."""
    sc = _Scan(d)
    polys, cur = [], None
    x = y = sx = sy = 0.0
    qx = qy = 0.0       # last control point, for S and T
    prev, cmd = '', None
    try:
        while sc.skip():
            ch = d[sc.i]
            if ch in CMDS:
                cmd = ch
                sc.i += 1
            elif ch.isalpha():
                raise ValueError('unknown command %r' % ch)
            elif cmd is None or cmd in 'Zz':
                raise ValueError('number without command at offset %d' % sc.i)
            lo = cmd.lower()
            if not prev and lo != 'm':
                raise ValueError('path does not start with M')
            ox, oy = (x, y) if cmd.islower() else (0.0, 0.0)
            if lo == 'z':
                if cur is not None and cur[-1] != cur[0]:
                    cur.append(cur[0])
                cur, x, y, prev = None, sx, sy, 'z'
                continue
            if lo == 'm':
                x = sx = sc.num() + ox
                y = sy = sc.num() + oy
                cur = [apply(ctm, x, y)]
                polys.append(cur)
                cmd, prev = ('l' if cmd == 'm' else 'L'), 'm'
                continue
            if cur is None:
                cur = [apply(ctm, x, y)]
                polys.append(cur)
            if lo == 'l':
                nx, ny = sc.num() + ox, sc.num() + oy
                cur.append(apply(ctm, nx, ny))
            elif lo == 'h':
                nx, ny = sc.num() + ox, y
                cur.append(apply(ctm, nx, ny))
            elif lo == 'v':
                nx, ny = x, sc.num() + oy
                cur.append(apply(ctm, nx, ny))
            elif lo in 'cs':
                if lo == 'c':
                    x1, y1 = sc.num() + ox, sc.num() + oy
                else:
                    x1, y1 = (2 * x - qx, 2 * y - qy) if prev in ('c', 's') else (x, y)
                x2, y2 = sc.num() + ox, sc.num() + oy
                nx, ny = sc.num() + ox, sc.num() + oy
                cubic(cur, ctm, x, y, x1, y1, x2, y2, nx, ny)
                qx, qy = x2, y2
            elif lo in 'qt':
                if lo == 'q':
                    x1, y1 = sc.num() + ox, sc.num() + oy
                else:
                    x1, y1 = (2 * x - qx, 2 * y - qy) if prev in ('q', 't') else (x, y)
                nx, ny = sc.num() + ox, sc.num() + oy
                quad(cur, ctm, x, y, x1, y1, nx, ny)
                qx, qy = x1, y1
            else:
                rx, ry, phi = sc.num(), sc.num(), sc.num()
                fa, fs = sc.flag(), sc.flag()
                nx, ny = sc.num() + ox, sc.num() + oy
                arc(cur, ctm, x, y, rx, ry, phi, fa, fs, nx, ny)
            x, y, prev = nx, ny, lo
    except ValueError as e:
        if warn:
            warn('path data: %s' % e)
    return [p for p in polys if len(p) > 1]


def flatten(el, tag, ctm, fs, warn):
    def L(name, default=0.0):
        return length(el.get(name), fs, default)
    if tag == 'path':
        return flatten_path(el.get('d') or '', ctm, warn)
    if tag in ('rect', 'image'):
        x, y, w, h = L('x'), L('y'), L('width'), L('height')
        if w <= 0 or h <= 0:
            return []
        if tag == 'rect' and (el.get('rx') or el.get('ry')):
            rx, ry = L('rx', -1.0), L('ry', -1.0)
            rx, ry = (ry if rx < 0 else rx), (rx if ry < 0 else ry)
            rx, ry = min(max(rx, 0.0), w / 2), min(max(ry, 0.0), h / 2)
            if rx > 0 and ry > 0:
                d = ('M%r,%r H%r A%r,%r 0 0 1 %r,%r V%r A%r,%r 0 0 1 %r,%r H%r A%r,%r 0 0 1 %r,%r V%r '
                     'A%r,%r 0 0 1 %r,%r Z' % (x + rx, y, x + w - rx, rx, ry, x + w, y + ry, y + h - ry,
                                               rx, ry, x + w - rx, y + h, x + rx, rx, ry, x, y + h - ry,
                                               y + ry, rx, ry, x + rx, y))
                return flatten_path(d, ctm, warn)
        return [[apply(ctm, px, py) for px, py in ((x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y))]]
    if tag in ('ellipse', 'circle'):
        cx, cy = L('cx'), L('cy')
        rx, ry = (L('r'), L('r')) if tag == 'circle' else (L('rx'), L('ry'))
        if rx <= 0 or ry <= 0:
            return []
        pts = arc_points(cx, cy, rx, ry, 0.0, 0.0, 2 * math.pi, ctm)
        pts[-1] = pts[0]
        return [pts]
    if tag == 'line':
        return [[apply(ctm, L('x1'), L('y1')), apply(ctm, L('x2'), L('y2'))]]
    v = [float(t) for t in NUM.findall(el.get('points') or '')]
    pts = [apply(ctm, v[i], v[i + 1]) for i in range(0, len(v) - 1, 2)]
    if tag == 'polygon' and len(pts) > 1:
        pts.append(pts[0])
    return [pts] if len(pts) > 1 else []


def text_info(el, ctm, fs, css):
    """(normalized content, anchor of the first character in page coordinates) or None."""
    pieces = []

    def walk(e, chain):
        if e.text:
            pieces.append((e.text, chain))
        for ch in e:
            ns, tag = split_tag(ch.tag)
            if ns in ('', SVG_NS) and tag in ('tspan', 'a', 'textPath') and not hidden(ch, css):
                if tag == 'tspan' and any(ch.get(k) is not None for k in ('x', 'y', 'dy')):
                    pieces.append((' ', None))      # a new line: keep the words apart
                walk(ch, chain + ((ch, font_size(ch, chain[-1][1], css)),))
            if ch.tail:
                pieces.append((ch.tail, chain))

    walk(el, ((el, fs),))
    content = ' '.join(''.join(p for p, _ in pieces).split())
    if not content:
        return None
    chain = next(c for p, c in pieces if c is not None and p.strip())
    pos = {'x': 0.0, 'y': 0.0, 'dx': 0.0, 'dy': 0.0}
    for e, efs in chain:            # outermost first, the innermost value wins
        for k in pos:
            v = first(e.get(k))
            if v is not None:
                pos[k] = length(v, efs)
    return content, apply(ctm, pos['x'] + pos['dx'], pos['y'] + pos['dy'])


# ---------------------------------------------------------------- loading

class Shape:
    __slots__ = ('ctx', 'id', 'kind', 'title', 'polys', 'texts')

    def __init__(self, ctx, sid, kind, title):
        self.ctx, self.id, self.kind, self.title = ctx, sid, kind, title
        self.polys, self.texts = [], []


class Doc:
    def __init__(self, name):
        self.name = name
        self.title = None       # title of the first foreground page group
        self.viewbox = None
        self.shapes = {}        # (context, id) -> Shape
        self.bgs = []           # background contexts in document order
        self.warnings = []

    def warn(self, msg):
        if msg not in self.warnings:
            self.warnings.append(msg)


def parse_xml(data):
    try:
        return ET.fromstring(data)
    except ET.ParseError as e:
        if 'unbound prefix' not in str(e):
            raise SvgError('XML: %s' % e)
    # tolerate a missing xmlns:v / xmlns:xlink declaration
    if isinstance(data, bytes):
        try:
            text = data.decode('utf-8')
        except UnicodeDecodeError:
            text = data.decode('latin-1')
    else:
        text = data
    text = re.sub(r'^\s*<\?xml[^>]*\?>', '', text)
    add = ''.join(' xmlns:%s="%s"' % (p, ns) for p, ns in (('v', VISIO_NS), ('xlink', XLINK_NS))
                  if 'xmlns:%s=' % p not in text)
    try:
        return ET.fromstring(re.sub(r'<svg\b', '<svg' + add, text, count=1))
    except ET.ParseError as e:
        raise SvgError('XML: %s' % e)


def load_svg(data, name='?'):
    root = parse_xml(data)
    if split_tag(root.tag)[1] != 'svg':
        raise SvgError('root element is not <svg>')
    doc = Doc(name)
    vb = NUM.findall(root.get('viewBox') or '')
    if len(vb) == 4:
        doc.viewbox = tuple(float(v) for v in vb)
    css = parse_css(root)
    nfg = unowned = 0
    stack = [(root, IDENT, '', None, 16.0)]
    while stack:
        el, ctm, ctx, owner, fs = stack.pop()
        ns, tag = split_tag(el.tag)
        if ns not in ('', SVG_NS) or tag in SKIP or el.get('data-jf') in ('arrow', 'shadow') or hidden(el, css):
            continue
        if tag == 'g' and (el.get('id') or '').lower().startswith('shadow'):
            continue
        tr = el.get('transform')
        if tr:
            m = parse_transform(tr)
            if m is None:
                doc.warn('malformed transform %r ignored' % tr)
            else:
                ctm = mul(ctm, m)
        fs = font_size(el, fs, css)
        if tag in ('g', 'svg', 'a', 'switch'):
            if tag == 'svg' and el is not root:
                ctm = mul(ctm, (1.0, 0.0, 0.0, 1.0, length(el.get('x'), fs), length(el.get('y'), fs)))
            gc = attr_local(el, 'groupContext') or ''
            mid = attr_local(el, 'mID')
            if mid is None:
                mid = el.get('data-mid')
            if gc.lower().endswith('page'):
                title = child_title(el) or ''
                if gc.lower().startswith('background'):
                    ctx = 'bg:' + title
                    if ctx not in doc.bgs:
                        doc.bgs.append(ctx)
                else:
                    nfg += 1
                    ctx = 'fg' if nfg == 1 else 'fg%d' % nfg
                    if doc.title is None:
                        doc.title = title
                owner = None
            elif mid is not None and mid.strip():
                key = (ctx, mid.strip())
                if key in doc.shapes:
                    doc.warn('shape ID %s appears twice (merged)' % key[1])
                else:
                    doc.shapes[key] = Shape(ctx, key[1], gc or 'shape', child_title(el))
                owner = doc.shapes[key]
            stack.extend((ch, ctm, ctx, owner, fs) for ch in reversed(list(el)))
        elif tag in GEOM or tag == 'text':
            if owner is None:
                unowned += 1
            elif tag == 'text':
                t = text_info(el, ctm, fs, css)
                if t:
                    owner.texts.append(t)
            else:
                owner.polys.extend(flatten(el, tag, ctm, fs,
                                           lambda m, s=owner: doc.warn('shape %s: %s' % (s.id, m))))
    if doc.title is None:
        doc.title = child_title(root)
    if unowned:
        doc.warn('%d geometry/text element(s) outside any shape ignored' % unowned)
    return doc


def load_file(path):
    try:
        with open(path, 'rb') as f:
            data = f.read()
    except OSError as e:
        raise SvgError(str(e))
    return load_svg(data, path)


# ---------------------------------------------------------------- distances

def bbox(polys):
    xs = [x for p in polys for x, _ in p]
    ys = [y for p in polys for _, y in p]
    return (min(xs), min(ys), max(xs), max(ys))


def seg_list(polys):
    out = []
    for p in polys:
        for (x0, y0), (x1, y1) in zip(p, p[1:]):
            dx, dy = x1 - x0, y1 - y0
            out.append((x0, y0, dx, dy, dx * dx + dy * dy))
    return out


def samples(polys, step):
    """Points along the polylines, at most step apart (by arc length), ends included."""
    total = sum(math.hypot(x1 - x0, y1 - y0) for p in polys for (x0, y0), (x1, y1) in zip(p, p[1:]))
    if total / step > MAX_SAMPLES:
        step = total / MAX_SAMPLES
    out = []
    for p in polys:
        x0, y0 = p[0]
        out.append((x0, y0))
        acc = 0.0                       # arc length since the last sample
        for x1, y1 in p[1:]:
            seg = math.hypot(x1 - x0, y1 - y0)
            pos = step - acc
            while pos < seg:
                f = pos / seg
                out.append((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f))
                pos += step
            acc = seg - (pos - step)
            x0, y0 = x1, y1
        out.append((x0, y0))
    return out


def brute_directed(pts, segs):
    """max over pts of the distance to the nearest segment; a point that cannot raise it stops early."""
    cmax2 = 0.0
    for px, py in pts:
        best = INF
        for x0, y0, dx, dy, l2 in segs:
            ex, ey = px - x0, py - y0
            if l2 > 0.0:
                t = (ex * dx + ey * dy) / l2
                t = 0.0 if t < 0.0 else (1.0 if t > 1.0 else t)
                ex -= t * dx
                ey -= t * dy
            d = ex * ex + ey * ey
            if d < best:
                best = d
                if best <= cmax2:
                    break
        if best > cmax2:
            cmax2 = best
    return math.sqrt(cmax2)


class Grid:
    """Segments split into pieces no longer than the cell size c, bucketed by cell; ring search."""

    def __init__(self, segs, c):
        x0 = min(min(s[0], s[0] + s[2]) for s in segs)
        y0 = min(min(s[1], s[1] + s[3]) for s in segs)
        cells = {}
        for sx, sy, dx, dy, l2 in segs:
            k = int(math.sqrt(l2) / c) + 1
            ex, ey = dx / k, dy / k
            piece_l2 = ex * ex + ey * ey
            for i in range(k):
                ax, ay = sx + dx * i / k, sy + dy * i / k
                bx, by = ax + ex, ay + ey
                piece = (ax, ay, ex, ey, piece_l2)
                lx, hx = (ax, bx) if ax <= bx else (bx, ax)
                ly, hy = (ay, by) if ay <= by else (by, ay)
                for gx in range(max(0, int((lx - x0) // c)), int((hx - x0) // c) + 1):
                    for gy in range(max(0, int((ly - y0) // c)), int((hy - y0) // c) + 1):
                        lst = cells.get((gx, gy))
                        if lst is None:
                            cells[(gx, gy)] = [piece]
                        else:
                            lst.append(piece)
        self.c, self.x0, self.y0, self.cells = c, x0, y0, cells
        self.nx = max(k[0] for k in cells) + 1
        self.ny = max(k[1] for k in cells) + 1

    def ring(self, ix, iy, r):
        nx, ny = self.nx, self.ny
        xa, xb = max(ix - r, 0), min(ix + r, nx - 1)
        for gy in (iy - r, iy + r):
            if 0 <= gy < ny:
                for gx in range(xa, xb + 1):
                    yield gx, gy
        ya, yb = max(iy - r + 1, 0), min(iy + r - 1, ny - 1)
        for gx in (ix - r, ix + r):
            if 0 <= gx < nx:
                for gy in range(ya, yb + 1):
                    yield gx, gy

    def directed(self, pts):
        """As brute_directed, scanning rings of cells around each point."""
        c, get, ring = self.c, self.cells.get, self.ring
        gx0, gy0, nx, ny = self.x0, self.y0, self.nx, self.ny
        cmax2 = 0.0
        for px, py in pts:
            ix, iy = int((px - gx0) // c), int((py - gy0) // c)
            r = 0 if 0 <= ix < nx and 0 <= iy < ny else max(-ix, ix - nx + 1, -iy, iy - ny + 1)
            rmax = max(ix, nx - 1 - ix, iy, ny - 1 - iy)
            best = INF
            while True:
                for key in (((ix, iy),) if r == 0 else ring(ix, iy, r)):
                    lst = get(key)
                    if not lst:
                        continue
                    for x0, y0, dx, dy, l2 in lst:
                        ex, ey = px - x0, py - y0
                        if l2 > 0.0:
                            t = (ex * dx + ey * dy) / l2
                            t = 0.0 if t < 0.0 else (1.0 if t > 1.0 else t)
                            ex -= t * dx
                            ey -= t * dy
                        d = ex * ex + ey * ey
                        if d < best:
                            best = d
                    if best <= cmax2:
                        break
                # every point closer than r * c lies in the rings scanned so far
                if best <= cmax2 or best <= (r * c) ** 2 or r >= rmax:
                    break
                r += 1
            if best > cmax2:
                cmax2 = best
        return math.sqrt(cmax2)


def directed(pa, pb, step, mode=None):
    """Largest distance from the samples of pa to the outline pb (mode: None, 'brute', 'grid')."""
    segs, pts = seg_list(pb), samples(pa, step)
    if not segs or not pts:
        return INF
    if mode == 'grid' or (mode is None and len(pts) * len(segs) > BRUTE):
        b = bbox(pb)
        return Grid(segs, max(2 * step, math.hypot(b[2] - b[0], b[3] - b[1]) / 64)).directed(pts)
    return brute_directed(pts, segs)


def hausdorff(pa, pb, step, mode=None):
    return max(directed(pa, pb, step, mode), directed(pb, pa, step, mode))


# ---------------------------------------------------------------- comparison

def skey(k):
    ctx, sid = k
    rank = 0 if ctx in ('', 'fg') else (1 if ctx.startswith('fg') else 2)
    return (rank, ctx, int(sid) if sid.isdigit() else 1 << 62, sid)


def label(k):
    ctx, sid = k
    if ctx.startswith('bg:'):
        return 'shape %s (background "%s")' % (sid, ctx[3:])
    if ctx not in ('', 'fg'):
        return 'shape %s (%s)' % (sid, ctx)
    return 'shape %s' % sid


def squash(s):
    return ''.join(s.split())


def compare_shape(a, b, k, opt):
    """a: ours, b: reference."""
    r = {'context': k[0], 'id': k[1], 'label': label(k), 'title': b.title or a.title, 'error': None,
         'tol': None, 'bbox': None, 'hausdorff': None, 'reason': None, 'geo_ok': True,
         'text': None, 'text_ok': True}
    if a.polys and b.polys:
        ba, bb = bbox(a.polys), bbox(b.polys)
        tol = opt.tol + opt.rel * math.hypot(bb[2] - bb[0], bb[3] - bb[1])
        dbox = max(abs(u - v) for u, v in zip(ba, bb))
        h = hausdorff(a.polys, b.polys, max(0.02, min(0.25, tol / 8)))
        r.update(error=max(dbox, h), tol=tol, bbox=dbox, hausdorff=h, geo_ok=dbox <= tol and h <= tol)
    elif a.polys or b.polys:
        r.update(reason='geometry only in %s' % ('ours' if a.polys else 'ref'), geo_ok=False)
    if a.texts or b.texts:
        ta = ' '.join(t for t, _ in a.texts)
        tb = ' '.join(t for t, _ in b.texts)
        if not a.texts or not b.texts:
            r['text'] = 'text only in %s: "%s"' % (('ours', ta) if a.texts else ('ref', tb))
        elif squash(ta) != squash(tb):
            r['text'] = '"%s" vs ref "%s"' % (ta, tb)
        else:
            (ax, ay), (bx, by) = a.texts[0][1], b.texts[0][1]
            d = math.hypot(ax - bx, ay - by)
            if d > opt.text_tol:
                r['text'] = 'anchor (%s, %s) vs ref (%s, %s), %s pt apart > %s' % (
                    fmt(ax), fmt(ay), fmt(bx), fmt(by), fmt(d), fmt(opt.text_tol))
        r['text_ok'] = r['text'] is None
    r['status'] = 'ok' if r['geo_ok'] and (r['text_ok'] or not opt.strict_text) else 'off'
    return r


def compare_docs(ours, ref, opt, ours_name='', ref_name=''):
    notes = []
    remap = {}
    if ours.bgs and set(ours.bgs) != set(ref.bgs) and len(ours.bgs) == len(ref.bgs):
        remap = dict(zip(ours.bgs, ref.bgs))
        notes.append('background pages paired by order: ' + ', '.join(
            'ours "%s" = ref "%s"' % (o[3:], f[3:]) for o, f in remap.items() if o != f))
    so = {(remap.get(c, c), i): s for (c, i), s in ours.shapes.items() if s.polys or s.texts}
    sr = {k: s for k, s in ref.shapes.items() if s.polys or s.texts}
    if ours.viewbox and ref.viewbox and max(abs(u - v) for u, v in zip(ours.viewbox, ref.viewbox)) > 0.5:
        notes.append('viewBox differs: ours %s, ref %s' % (' '.join(map(fmt, ours.viewbox)),
                                                          ' '.join(map(fmt, ref.viewbox))))
    results = [compare_shape(so[k], sr[k], k, opt) for k in sorted(set(so) & set(sr), key=skey)]
    missing = [{'context': k[0], 'id': k[1], 'label': label(k), 'title': sr[k].title}
               for k in sorted(set(sr) - set(so), key=skey)]
    extra = [{'context': k[0], 'id': k[1], 'label': label(k), 'title': so[k].title}
             for k in sorted(set(so) - set(sr), key=skey)]
    measured = [r for r in results if r['error'] is not None]
    worst = max(measured, key=lambda r: r['error']) if measured else None
    n_off = sum(r['status'] == 'off' for r in results)
    return {
        'page': ref.title or ours.title or os.path.splitext(os.path.basename(ref_name))[0] or '?',
        'ours': ours_name, 'ref': ref_name,
        'shapes': len(results), 'ok': len(results) - n_off, 'off': n_off,
        'missing': len(missing), 'extra': len(extra),
        'text_mismatches': sum(r['text'] is not None for r in results),
        'worst': worst['error'] if worst else None, 'worst_shape': worst['label'] if worst else None,
        'pass': n_off == 0 and not missing and not extra,
        'results': results, 'missing_shapes': missing, 'extra_shapes': extra, 'notes': notes,
        'warnings': ['ours: ' + w for w in ours.warnings] + ['ref: ' + w for w in ref.warnings],
    }


# ---------------------------------------------------------------- report

def fmt(v):
    s = ('%.2f' % v).rstrip('0').rstrip('.')
    return '0' if s in ('', '-0') else s


def summary(p):
    s = 'page "%s": %d shapes, %d ok, %d off, %d missing, %d extra, ' % (
        p['page'], p['shapes'], p['ok'], p['off'], p['missing'], p['extra'])
    s += 'worst -' if p['worst'] is None else 'worst %s pt (%s)' % (fmt(p['worst']), p['worst_shape'])
    n = p['text_mismatches']
    if n:
        s += ', %d text mismatch%s' % (n, '' if n == 1 else 'es')
    return s


def details(p):
    def name(r):
        return r['label'] + (' "%s"' % r['title'] if r.get('title') else '')
    out = []
    bad = [r for r in p['results'] if not r['geo_ok']]
    bad.sort(key=lambda r: -(INF if r['error'] is None else r['error']))
    for r in bad:
        if r['error'] is None:
            out.append('  off      %s: %s' % (name(r), r['reason']))
        else:
            out.append('  off      %s: %s pt > %s (bbox %s, hausdorff %s)' % (
                name(r), fmt(r['error']), fmt(r['tol']), fmt(r['bbox']), fmt(r['hausdorff'])))
    out += ['  missing  %s' % name(m) for m in p['missing_shapes']]
    out += ['  extra    %s' % name(m) for m in p['extra_shapes']]
    out += ['  text     %s: %s' % (name(r), r['text']) for r in p['results'] if r['text']]
    out += ['  note     ' + n for n in p['notes']]
    out += ['  warning  ' + w for w in p['warnings']]
    return out


def report(pages, lost, errors, opt):
    for p in pages:
        print(summary(p))
    for name in lost:
        print('page file "%s": no SVG of ours for it' % name)
    for path, msg in errors:
        print('error: %s: %s' % (path, msg))
    for p in pages:
        lines = details(p)
        if not lines or opt.max_report <= 0:
            continue
        print('details of page "%s" (ours %s, ref %s):' % (p['page'], p['ours'], p['ref']))
        for line in lines[:opt.max_report]:
            print(line)
        if len(lines) > opt.max_report:
            print('  ... %d more' % (len(lines) - opt.max_report))
    failed = sum(not p['pass'] for p in pages) + len(lost)
    print('%d page(s) compared, %d failing%s' % (len(pages), failed,
                                                  ', %d parse error(s)' % len(errors) if errors else ''))
    return 2 if errors else (1 if failed else 0)


# ---------------------------------------------------------------- folders

PAGE_FILE = re.compile(r'^(.*)\.p(\d+)\.svg$', re.I)


def page_key(name):
    m = PAGE_FILE.match(name)
    if m:
        return (m.group(1).lower(), int(m.group(2)))
    return (name[:-4].lower(), 1)


def pair_files(ref_names, ours_names, title_of):
    """Pairs reference files with ours by <stem>.p<N>.svg, else by page title.
    title_of(side, name) -> page title or None. Returns ([(ref, ours)], [unpaired ref])."""
    by_key = {}
    for n in sorted(ours_names, key=lambda n: PAGE_FILE.match(n) is not None):
        by_key[page_key(n)] = n
    found = {r: by_key.get(page_key(r)) for r in ref_names}
    used = set(o for o in found.values() if o)
    for r in sorted(ref_names):
        t = title_of('ref', r) if found[r] is None else None
        if t:
            cands = [n for n in sorted(ours_names) if n not in used and title_of('ours', n) == t]
            same = [n for n in cands if page_key(n)[0] == page_key(r)[0]]
            o = same[0] if len(same) == 1 else (cands[0] if len(cands) == 1 else None)
            if o:
                found[r] = o
                used.add(o)
    pairs = [(r, found[r]) for r in sorted(ref_names) if found[r]]
    return pairs, [r for r in sorted(ref_names) if not found[r]]


def svg_names(folder):
    return [n for n in os.listdir(folder)
            if n.lower().endswith('.svg') and os.path.isfile(os.path.join(folder, n))]


# ---------------------------------------------------------------- self-test

_VISIO = '''<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<!-- synthetic, laid out like Visio's SVG export -->
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
    xmlns:ev="http://www.w3.org/2001/xml-events" xmlns:v="http://schemas.microsoft.com/visio/2003/SVGExtensions/"
    width="11.6929in" height="8.26772in" viewBox="0 0 841.89 595.276" xml:space="preserve"
    color-interpolation-filters="sRGB" class="st9">
  <v:documentProperties v:langID="1033" v:viewMarkup="false"/>
  <style type="text/css"><![CDATA[
    .st1 {fill:#ffffff;stroke:#000000;stroke-width:0.75}
    .st2 {fill:#000000;font-family:Calibri;font-size:1.2em}
    .st3 {marker-end:url(#mrkr4-7);stroke:#000000}
    .st9 {fill:none;fill-rule:evenodd;font-size:12px;overflow:visible}
  ]]></style>
  <defs id="Markers">
    <g id="lend4"><path d="M 2 1 L 0 0 L 2 -1 L 2 1" style="stroke:none"/></g>
    <marker id="mrkr4-7" refX="-7.04" orient="auto" markerUnits="strokeWidth" overflow="visible">
      <use xlink:href="#lend4" transform="scale(-3.52,-3.52) "/>
    </marker>
    <g v:mID="77" v:groupContext="shape"><rect x="0" y="0" width="5" height="5"/></g>
  </defs>
  <g v:mID="4" v:index="2" v:groupContext="backgroundPage">
    <title>Background-1</title>
    <g id="shape1-1" v:mID="1" v:groupContext="shape" transform="translate(18,-560)">
      <title>Sheet.1</title>
      <rect x="0" y="575.276" width="100" height="20" class="st1"/>
    </g>
  </g>
  <g v:mID="0" v:index="1" v:groupContext="foregroundPage">
    <title>Page-1</title>
    <v:pageProperties v:drawingScale="1" v:pageScale="1" v:drawingUnits="19"/>
    <g id="shape1-2" v:mID="1" v:groupContext="shape" transform="translate(20,-500)">
      <title>Box</title><desc>Box</desc>
      <v:userDefs><v:ud v:nameU="x" v:val="VT0(1):26"/></v:userDefs>
      <rect x="0" y="555.276" width="80" height="40" class="st1"/>
    </g>
    <g id="group2-3" v:mID="2" v:groupContext="group" transform="translate(200,-250) rotate(-30)">
      <title>Group 2</title>
      <path d="M0 595.276 L120 595.276" class="st1"/>
      <g id="shape3-4" v:mID="3" v:groupContext="shape" transform="translate(10,-60)">
        <title>Rect</title>
        <v:textBlock v:margins="rect(4,4,4,4)"/><v:textRect cx="50" cy="575.276" width="100" height="40"/>
        <rect x="0" y="555.276" width="100" height="40" class="st1"/>
        <text x="30" y="572" class="st2" v:langID="1033"><v:paragraph v:horizAlign="1"/><v:tabList/>Hello<tspan
            x="30" dy="1.2em" class="st2">World</tspan></text>
      </g>
      <g id="shape4-5" v:mID="4" v:groupContext="shape" transform="translate(20,-120) rotate(45 30 585)">
        <path d="M0 595.276 C 10 580 30 580 40 595.276 A 20 10 30 0 1 80 595.276 a10 10 0 1 0 -20 0 z" class="st1"/>
      </g>
    </g>
    <g id="shape5-6" v:mID="5" v:groupContext="shape" transform="translate(400,-200)">
      <title>Ellipse</title>
      <g id="shadow5-7" transform="matrix(1,0,0,1,2,2)" class="st1"><ellipse cx="30" cy="560" rx="30" ry="15"/></g>
      <ellipse cx="30" cy="560" rx="30" ry="15" class="st1"/>
    </g>
    <g id="shape6-8" v:mID="6" v:groupContext="shape" transform="translate(500,-100) scale(1.5)">
      <polyline points="0,300 10,280 20,300 30,280" class="st1"/>
    </g>
    <g id="shape7-9" v:mID="7" v:groupContext="shape" transform="translate(50,-50)">
      <title>Dynamic connector</title>
      <path d="M0 400 L100 300" class="st3" marker-end="url(#mrkr4-7)"/>
    </g>
    <g id="shape8-10" v:mID="8" v:groupContext="shape">
      <text x="300" y="100" class="st2"><tspan dy="-0.5em">Note</tspan></text>
    </g>
    <g id="shape9-11" v:mID="9" v:groupContext="shape" transform="matrix(1,0,0,1,600,-500)">
      <circle cx="0" cy="560" r="10" class="st1"/>
      <line x1="0" y1="560" x2="20" y2="560" class="st1"/>
    </g>
  </g>
</svg>
'''


def _selftest_ours(shift5=0.0, drop6=False, extra=False, hallo=False, bgname='Background-1'):
    """The page of _VISIO the way our test app writes it: absolute coordinates, computed here with
    plain point functions (not the matrix code under test)."""
    H = 595.276

    def T(tx, ty):
        return lambda p: (p[0] + tx, p[1] + ty)

    def R(deg, cx=0.0, cy=0.0):
        a = math.radians(deg)
        c, s = math.cos(a), math.sin(a)
        return lambda p: (cx + (p[0] - cx) * c - (p[1] - cy) * s, cy + (p[0] - cx) * s + (p[1] - cy) * c)

    def S(k):
        return lambda p: (p[0] * k, p[1] * k)

    def chain(*fs):             # outermost first, like a transform attribute
        def f(p):
            for g in reversed(fs):
                p = g(p)
            return p
        return f

    def P(f, x, y):
        return '%.9f,%.9f' % f((x, y))

    G = chain(T(200, -250), R(-30))
    M3 = chain(G, T(10, -60))
    M4 = chain(G, T(20, -120), R(45, 30, 585))
    M6 = chain(T(500, -100), S(1.5))
    ax, ay = M3((30, 572))
    corners = ' '.join(P(M3, x, y) for x, y in ((0, 555.276), (100, 555.276), (100, H), (0, H)))
    s = ['<svg xmlns="http://www.w3.org/2000/svg" xmlns:v="%s" viewBox="0 0 841.89 595.276">' % VISIO_NS,
         '<g v:groupContext="backgroundPage"><title>%s</title>' % bgname,
         '<g v:mID="1" v:groupContext="shape"><rect x="18" y="15.276" width="100" height="20"/></g></g>',
         '<g v:groupContext="foregroundPage"><title>Page-1</title>',
         '<g v:mID="1" v:groupContext="shape"><rect x="20" y="55.276" width="80" height="40"/></g>',
         '<g v:mID="2" v:groupContext="group"><path d="M%s L%s"/>' % (P(G, 0, H), P(G, 120, H)),
         '<g v:mID="3" v:groupContext="shape"><polygon points="%s"/>' % corners,
         '<text x="%.9f" y="%.9f" transform="rotate(-30 %.9f %.9f)"><tspan x="%.9f" y="%.9f">%s</tspan>'
         '<tspan x="%.9f" dy="14.4">World</tspan></text></g>' % (ax, ay, ax, ay, ax, ay,
                                                                  'Hallo' if hallo else 'Hello', ax),
         '<g v:mID="4" v:groupContext="shape"><path d="M%s C%s %s %s A20 10 45 0 1 %s A10 10 15 1 0 %s Z"/></g>'
         % (P(M4, 0, H), P(M4, 10, 580), P(M4, 30, 580), P(M4, 40, H), P(M4, 80, H), P(M4, 60, H)),
         '</g>',
         '<g v:mID="5" v:groupContext="shape"><ellipse cx="%r" cy="360" rx="30" ry="15"/></g>' % (430 + shift5)]
    if not drop6:
        s.append('<g v:mID="6" v:groupContext="shape"><polyline points="%s"/></g>'
                 % ' '.join(P(M6, x, y) for x, y in ((0, 300), (10, 280), (20, 300), (30, 280))))
    s += ['<g v:mID="7" v:groupContext="shape"><path d="M50,350 L150,250"/>',
          '<path data-jf="arrow" d="M150,250 L140,252 L148,242 Z"/></g>',
          '<g v:mID="8" v:groupContext="shape"><text x="300" y="92.8">Note</text></g>',
          '<g data-mid="9" v:groupContext="shape"><circle cx="600" cy="60" r="10"/>',
          '<line x1="600" y1="60" x2="620" y2="60"/></g>']
    if extra:
        s.append('<g v:mID="99" v:groupContext="shape"><rect x="700" y="500" width="10" height="10"/></g>')
    s.append('</g></svg>')
    return '\n'.join(s)


def selftest():
    fails, count = [], [0]

    def check(name, cond, detail=''):
        count[0] += 1
        if not cond:
            fails.append('%s %s' % (name, detail))

    def near(p, q, eps=1e-6):
        return abs(p[0] - q[0]) <= eps and abs(p[1] - q[1]) <= eps

    def bb_near(b, want, eps=0.01):
        return all(abs(u - v) <= eps for u, v in zip(b, want))

    def bb_of(d):
        return bbox(flatten_path(d, IDENT))

    # transforms
    check('rotate', near(apply(parse_transform('rotate(90)'), 1, 0), (0, 1)))
    check('rotate about a point', near(apply(parse_transform('rotate(90, 10, 0)'), 20, 0), (10, 10)))
    check('transform order', near(apply(parse_transform('translate(5,0) rotate(90)'), 1, 0), (5, 1)))
    check('scale', near(apply(parse_transform('scale(2) translate(1 1)'), 0, 0), (2, 2)))
    check('matrix, skewX', near(apply(parse_transform('matrix(1 0 0 1 3 4) skewX(45)'), 0, 1), (4, 5)))
    check('malformed transform', parse_transform('rotate(30') is None and parse_transform('turn(3)') is None)
    # paths and arcs
    pts = flatten_path('M0 0 A10 10 0 0 1 20 0', IDENT)[0]
    check('arc half circle', bb_near(bbox([pts]), (0, -10, 20, 0)), str(bbox([pts])))
    check('arc points on the circle', max(abs(math.hypot(x - 10, y) - 10) for x, y in pts) < 1e-9)
    check('arc radius scaled up', bb_near(bb_of('M0 0 A1 1 0 0 0 20 0'), (0, 0, 20, 10)))
    check('arc large flag', bb_near(bb_of('M0 0 A10 10 0 1 1 10 10'), (0, -10, 20, 10)))
    check('arc rotated ellipse', bb_near(bb_of('M0 0 A20 10 90 0 1 0 40'), (0, 0, 10, 40)))
    check('arc packed flags', bb_near(bb_of('M0 0a10 10 0 0120 0'), (0, -10, 20, 0)))
    pts = flatten_path('M0 0 A100 100 0 0 1 200 0', IDENT)[0]
    sag = max(100 - math.hypot((x0 + x1) / 2 - 100, (y0 + y1) / 2)
              for (x0, y0), (x1, y1) in zip(pts, pts[1:]))
    check('arc chord deviation', 0 < sag <= FLAT * 1.001, str(sag))
    check('cubic', bb_near(bb_of('M0 0 C0 10 10 10 10 0'), (0, 0, 10, 7.5)))
    check('smooth cubic', bb_near(bb_of('M0 0 C0 10 10 10 10 0 S20 -10 20 0'), (0, -7.5, 20, 7.5)))
    check('quadratic and T', bb_near(bb_of('M0 0 Q5 10 10 0 T20 0'), (0, -5, 20, 5)))
    check('relative h v z', bb_near(bb_of('m10 10 h10 v10 H10 z'), (10, 10, 20, 20)))
    rr = flatten(ET.fromstring('<rect x="0" y="0" width="20" height="10" rx="2"/>'), 'rect', IDENT, 16.0, None)
    plain = flatten(ET.fromstring('<rect x="0" y="0" width="20" height="10"/>'), 'rect', IDENT, 16.0, None)
    check('rounded rect', bb_near(bbox(rr), (0, 0, 20, 10))
          and abs(hausdorff(rr, plain, 0.05) - 2 * (math.sqrt(2) - 1)) < 0.03, str(hausdorff(rr, plain, 0.05)))
    msgs = []
    check('path error keeps the start',
          len(flatten_path('M0 0 L10 0 L5', IDENT, msgs.append)[0]) == 2 and msgs)
    # distances
    sq = [[(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)]]
    check('hausdorff of a shift', abs(hausdorff(sq, [[(x + 3, y) for x, y in sq[0]]], 0.1) - 3) < 0.01)
    import random
    rnd = random.Random(7)
    for i, off in enumerate(((0, 0), (0, 0), (250, -150))):
        a = [[(rnd.uniform(0, 300), rnd.uniform(0, 200)) for _ in range(60)]]
        b = [[(x + off[0] + rnd.uniform(-2, 2), y + off[1] + rnd.uniform(-2, 2)) for x, y in a[0]]]
        hb, hg = hausdorff(a, b, 0.5, 'brute'), hausdorff(a, b, 0.5, 'grid')
        check('grid = brute %d' % i, abs(hb - hg) < 1e-6, '%r %r' % (hb, hg))
    # documents
    opt = argparse.Namespace(tol=1.5, rel=0.01, text_tol=6.0, strict_text=False)
    ref = load_svg(_VISIO.encode('utf-8'), 'visio')

    def cmp(o=None, **kw):
        return compare_docs(load_svg(_selftest_ours(**kw), 'ours'), ref, o or opt)

    ours = load_svg(_selftest_ours(), 'ours')
    p = compare_docs(ours, ref, opt)

    def R(i, ctx='fg'):
        return ref.shapes[(ctx, i)]

    check('identical passes', p['pass'] and p['shapes'] == 10 and p['ok'] == 10, summary(p))
    check('identical error < 0.01', p['worst'] is not None and p['worst'] < 0.01, str(p['worst']))
    check('summary line', re.fullmatch(r'page "Page-1": 10 shapes, 10 ok, 0 off, 0 missing, 0 extra, '
                                       r'worst [0-9.]+ pt \(shape \d+.*\)', summary(p)), summary(p))
    check('defs ignored', all(k[1] != '77' for k in ref.shapes) and len(ref.shapes) == 10,
          str(list(ref.shapes)))
    check('markers and arrows ignored', bb_near(bbox(R('7').polys), (50, 250, 150, 350), 1e-6)
          and bb_near(bbox(ours.shapes[('fg', '7')].polys), (50, 250, 150, 350), 1e-6))
    check('shadow group skipped', len(R('5').polys) == 1
          and bb_near(bbox(R('5').polys), (400, 345, 460, 375), FLAT * 1.5))
    check('background IDs apart',
          bb_near(bbox(R('1', 'bg:Background-1').polys), (18, 15.276, 118, 35.276), 1e-6)
          and bb_near(bbox(R('1').polys), (20, 55.276, 100, 95.276), 1e-6))
    check('group keeps its own geometry', len(R('2').polys) == 1)
    check('text content', R('3').texts[0][0] == 'Hello World', repr(R('3').texts))
    check('text em offset', len(R('8').texts) == 1 and R('8').texts[0][0] == 'Note'
          and near(R('8').texts[0][1], (300, 92.8), 1e-9), repr(R('8').texts))
    check('text anchor under rotation', near(R('3').texts[0][1], ours.shapes[('fg', '3')].texts[0][1]))
    check('data-mid', ('fg', '9') in ours.shapes)
    check('json', json.loads(json.dumps(p))['shapes'] == 10)
    p = cmp(shift5=3.0)
    off = [r for r in p['results'] if r['status'] == 'off']
    check('shift fails that shape only', len(off) == 1 and off[0]['id'] == '5'
          and abs(off[0]['error'] - 3) < 0.01 and p['worst_shape'] == 'shape 5', summary(p))
    p = cmp(drop6=True)
    check('missing shape', [m['id'] for m in p['missing_shapes']] == ['6'] and p['off'] == 0 and not p['pass'],
          summary(p))
    p = cmp(extra=True)
    check('extra shape', [m['id'] for m in p['extra_shapes']] == ['99'] and p['off'] == 0 and not p['pass'],
          summary(p))
    p = cmp(hallo=True)
    t = [r for r in p['results'] if r['text']]
    check('text mismatch reported', len(t) == 1 and t[0]['id'] == '3' and p['pass'], summary(p))
    p = cmp(argparse.Namespace(tol=1.5, rel=0.01, text_tol=6.0, strict_text=True), hallo=True)
    check('strict text fails', not p['pass'] and p['off'] == 1, summary(p))
    p = cmp(bgname='Hintergrund-1')
    check('background pages paired by order', p['pass'] and any('order' in n for n in p['notes']), summary(p))
    d = load_svg(b'<svg xmlns="http://www.w3.org/2000/svg"><g v:mID="1"><rect width="1" height="1"/></g></svg>')
    check('undeclared v: prefix', ('', '1') in d.shapes)
    try:
        load_svg(b'<svg')
        check('parse error', False)
    except SvgError:
        check('parse error', True)
    titles = {('ref', 'c.p1.svg'): 'Seite X', ('ours', 'x.p3.svg'): 'Seite X', ('ref', 'd.p1.svg'): 'Nix',
              ('ref', '0.p2.svg'): 'Same', ('ours', 'a.p1.svg'): 'Same', ('ours', 'z.p9.svg'): 'Same'}
    pairs, lost = pair_files(['a.p1.svg', 'a.P2.svg', 'b.svg', 'c.p1.svg', 'd.p1.svg', '0.p2.svg'],
                             ['a.p1.svg', 'a.p2.svg', 'b.p1.svg', 'x.p3.svg', 'y.p1.svg', 'z.p9.svg'],
                             lambda side, n: titles.get((side, n)))
    check('pairing by name, then title', sorted(pairs) == [('0.p2.svg', 'z.p9.svg'), ('a.P2.svg', 'a.p2.svg'),
                                                           ('a.p1.svg', 'a.p1.svg'), ('b.svg', 'b.p1.svg'),
                                                           ('c.p1.svg', 'x.p3.svg')]
          and lost == ['d.p1.svg'], repr((pairs, lost)))
    if fails:
        for f in fails:
            print('selftest FAILED: ' + f)
        print('selftest: %d of %d failed' % (len(fails), count[0]))
        return 1
    print('selftest: %d ok' % count[0])
    return 0


# ---------------------------------------------------------------- main

def main(argv=None):
    try:
        sys.stdout.reconfigure(errors='replace')
    except (AttributeError, ValueError):
        pass
    ap = argparse.ArgumentParser(
        description="Compare our debug SVG of a page with Visio's SVG export, shape by shape.")
    ap.add_argument('files', nargs='*', metavar='SVG', help='OURS.svg REF.svg')
    ap.add_argument('--ours', metavar='DIR', help='folder of our SVGs (<stem>.p<N>.svg)')
    ap.add_argument('--ref', metavar='DIR', help='folder of Visio exports')
    ap.add_argument('--tol', type=float, default=1.5, help='absolute tolerance, pt (1.5)')
    ap.add_argument('--rel', type=float, default=0.01, help='tolerance relative to the bbox diagonal (0.01)')
    ap.add_argument('--text-tol', type=float, default=6.0, help='text anchor tolerance, pt (6)')
    ap.add_argument('--strict-text', action='store_true', help='text differences fail')
    ap.add_argument('--json', metavar='OUT', help='write the results as JSON')
    ap.add_argument('--max-report', type=int, default=30, help='detail lines per page (30)')
    ap.add_argument('--selftest', action='store_true', help='run the built-in checks')
    a = ap.parse_args(argv)
    if a.selftest:
        return selftest()
    jobs, lost, errors = [], [], []
    if a.files:
        if len(a.files) != 2 or a.ours or a.ref:
            ap.error('give OURS.svg REF.svg, or --ours DIR --ref DIR')
        jobs.append((a.files[0], a.files[1]))
    elif a.ours and a.ref:
        refs = svg_names(a.ref) if os.path.isdir(a.ref) else []
        if not refs:
            print('no reference SVGs found')
            return 0
        if not os.path.isdir(a.ours):
            print('error: folder of ours not found: %s' % a.ours, file=sys.stderr)
            return 2
        cache = {}

        def get(path):
            if path not in cache:
                try:
                    cache[path] = load_file(path)
                except SvgError as e:
                    cache[path] = e
            return cache[path]

        def title_of(side, name):
            d = get(os.path.join(a.ref if side == 'ref' else a.ours, name))
            return None if isinstance(d, SvgError) else d.title

        pairs, lost = pair_files(refs, svg_names(a.ours), title_of)
        jobs = [(os.path.join(a.ours, o), os.path.join(a.ref, r)) for r, o in pairs]
    else:
        ap.error('give OURS.svg REF.svg, or --ours DIR --ref DIR, or --selftest')
    pages = []
    for ours_path, ref_path in jobs:
        docs = []
        for path in (ours_path, ref_path):
            try:
                docs.append(load_file(path))
            except SvgError as e:
                errors.append((path, str(e)))
        if len(docs) == 2:
            pages.append(compare_docs(docs[0], docs[1], a, ours_path, ref_path))
    code = report(pages, lost, errors, a)
    if a.json:
        out = {'pass': code == 0, 'options': {'tol': a.tol, 'rel': a.rel, 'text_tol': a.text_tol,
                                              'strict_text': a.strict_text},
               'pages': pages, 'unpaired_ref': lost, 'errors': [{'file': f, 'error': e} for f, e in errors]}
        with open(a.json, 'w', encoding='utf-8') as f:
            json.dump(out, f, indent=1, ensure_ascii=False)
    return code


if __name__ == '__main__':
    sys.exit(main())
