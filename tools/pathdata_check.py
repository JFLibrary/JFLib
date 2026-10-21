#!/usr/bin/env python3
"""pathdata_check.py -- check gfx/pathdata.s (pd_parse, pd_emit, pd_bounds) against a Python
reference (agent T4).

  python tools/pathdata_check.py [--target win|linux|both] [--out build/t4] [--fuzz N] [--keep]

Runs the rstest ops `pdraster` and `pdfuzz` (apps/rstest/src/ops_dash.s):
* parse cases: SVG / XAML path data (all commands, relative / absolute, implicit repetition,
  compact numbers, exponents, F0 / F1, arcs incl. rotation / radius scaling / flags) are
  filled with their rule into a mask and compared with a reference rasterisation of the
  Python parser's geometry (arcs sampled on the exact ellipse, curves finely flattened;
  coverage by 16 x 16 supersampling with the winding rule).  Tolerance: |d| <= 24 / 255
  per pixel (supersampling error on edges) and mean |d| <= 0.8; bounds within 0.05 px
  (pd_bounds flattens with 0.02 units), the command count exact.
* error cases: the exact error code and byte offset of pd_parse.
* fuzz: N random strings (mutations of the cases, random command / number soup, truncated
  prefixes) each copied to the end of a page mapping (reads past it would fault), parsed,
  emitted, dash-stroked, rasterized; every line must report a result, and the result
  (ok / error class) must agree with the Python parser.
* --xaml N (default 40, 0 = off; needs build/.../PaintCraft): N mutated ShapeKit XAML files
  (apps/PaintCraft/tests/fixtures/shapes + random geometry) are loaded by PaintCraft's shape
  library (hook cmd 795 8<<16|2, folder in PDN_SHAPES_DIR) and every loaded shape is drawn;
  the script must end normally.
"""

import argparse
import math
import os
import random
import re
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from resample_check import read_raw, Runner  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
PDE_SYNTAX, PDE_NUMBER, PDE_RANGE = -40, -41, -42
COORD_MAX = 1e7
SS = 16

# ------------------------------------------------------------------ reference parser


class PErr(Exception):
    def __init__(self, code, off):
        super().__init__(code, off)
        self.code, self.off = code, off


NUM_RE = re.compile(rb"[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?")
WS = b" \t\n\r\x0c"
NARGS = {b"m": 2, b"l": 2, b"h": 1, b"v": 1, b"c": 6, b"s": 4, b"q": 4, b"t": 2, b"a": 7, b"z": 0}


def number(s, p):
    m = NUM_RE.match(s, p)
    if not m:
        raise PErr(PDE_NUMBER, p)
    txt = m.group(0)
    # the assembly keeps 18 significant digits and rejects exponents beyond +-22 of a
    # non-zero mantissa before |v| > 1e9: both give PDE_RANGE for such inputs
    v = float(txt)
    if abs(v) > 1e9 or math.isinf(v):
        raise PErr(PDE_RANGE, m.end())
    return v, m.end()


def skip_ws(s, p):
    while p < len(s) and s[p] in WS:
        p += 1
    return p


def sep(s, p):
    p = skip_ws(s, p)
    if p < len(s) and s[p] == 44:
        p = skip_ws(s, p + 1)
    return p


def arc_points(x1, y1, rx, ry, phi, fa, fs, x2, y2, n=256):
    """points on the exact SVG arc (excluding the start point)"""
    if rx == 0 or ry == 0:
        return [(x2, y2)]
    if x1 == x2 and y1 == y2:
        return []
    rx, ry = abs(rx), abs(ry)
    ph = math.radians(phi)
    c, s_ = math.cos(ph), math.sin(ph)
    dx2, dy2 = (x1 - x2) / 2, (y1 - y2) / 2
    x1p = c * dx2 + s_ * dy2
    y1p = -s_ * dx2 + c * dy2
    lam = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry)
    if lam > 1:
        rx *= math.sqrt(lam)
        ry *= math.sqrt(lam)
    num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    co = math.sqrt(max(0.0, num / den)) if den > 0 else 0.0
    if fa == fs:
        co = -co
    cxp = co * rx * y1p / ry
    cyp = -co * ry * x1p / rx
    cx = c * cxp - s_ * cyp + (x1 + x2) / 2
    cy = s_ * cxp + c * cyp + (y1 + y2) / 2
    ux, uy = (x1p - cxp) / rx, (y1p - cyp) / ry
    vx, vy = (-x1p - cxp) / rx, (-y1p - cyp) / ry
    t1 = math.atan2(uy, ux)
    dt = math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
    if not fs and dt > 0:
        dt -= 2 * math.pi
    elif fs and dt < 0:
        dt += 2 * math.pi
    pts = []
    for i in range(1, n + 1):
        t = t1 + dt * i / n
        pts.append((cx + rx * math.cos(t) * c - ry * math.sin(t) * s_,
                    cy + rx * math.cos(t) * s_ + ry * math.sin(t) * c))
    pts[-1] = (x2, y2)
    return pts


def cubic_pts(p0, c1, c2, p3, n=64):
    out = []
    for i in range(1, n + 1):
        t = i / n
        s_ = 1 - t
        out.append((s_ ** 3 * p0[0] + 3 * s_ * s_ * t * c1[0] + 3 * s_ * t * t * c2[0] + t ** 3 * p3[0],
                    s_ ** 3 * p0[1] + 3 * s_ * s_ * t * c1[1] + 3 * s_ * t * t * c2[1] + t ** 3 * p3[1]))
    return out


def parse(text):
    """-> (rule, [(points, closed)], ncmd) or raises PErr"""
    s = text.encode() if isinstance(text, str) else text
    rule = 0
    subs = []
    cur_pts = None
    ncmd = 0
    cx = cy = sx = sy = 0.0
    lk, lcx, lcy = 0, 0.0, 0.0

    def ck(*vals):
        for v in vals:
            if not math.isfinite(v) or abs(v) > COORD_MAX:
                raise PErr(PDE_RANGE, p)

    def open_sub():
        nonlocal cur_pts, ncmd
        if cur_pts is None:
            cur_pts = [(cx, cy)]
            subs.append([cur_pts, False])
            ncmd += 1

    p = skip_ws(s, 0)
    if p < len(s) and s[p] in b"fF":
        p = skip_ws(s, p + 1)
        if p >= len(s) or s[p] not in b"01":
            raise PErr(PDE_SYNTAX, p)
        rule = 1 if s[p] == 48 else 0
        p += 1
    cmd = None
    rel = False
    while True:
        p = skip_ws(s, p)
        if p >= len(s):
            break
        ch = bytes([s[p] | 32])
        if ch not in NARGS:
            raise PErr(PDE_SYNTAX, p)
        rel = s[p] >= 97
        if cmd is None and ch != b"m":
            raise PErr(PDE_SYNTAX, p)
        cmd = ch
        p += 1
        if cmd == b"z":
            if cur_pts is not None:
                subs[-1][1] = True
                ncmd += 1
                cur_pts = None
            cx, cy = sx, sy
            lk = 0
            continue
        while True:
            args = []
            for i in range(NARGS[cmd]):
                p = sep(s, p)
                if cmd == b"a" and i in (3, 4):
                    if p >= len(s) or s[p] not in b"01":
                        raise PErr(PDE_NUMBER, p)
                    args.append(float(s[p] - 48))
                    p += 1
                else:
                    v, p = number(s, p)
                    args.append(v)
            if rel:
                if cmd == b"h":
                    args[0] += cx
                elif cmd == b"v":
                    args[0] += cy
                elif cmd == b"a":
                    args[5] += cx
                    args[6] += cy
                else:
                    for i in range(0, len(args), 2):
                        args[i] += cx
                        args[i + 1] += cy
            if cmd == b"m":
                ck(args[0], args[1])
                cx, cy = args
                sx, sy = cx, cy
                cur_pts = [(cx, cy)]
                subs.append([cur_pts, False])
                ncmd += 1
                cmd = b"l"
                lk = 0
            elif cmd in (b"l", b"h", b"v"):
                x, y = (args[0], cy) if cmd == b"h" else (cx, args[0]) if cmd == b"v" else args
                ck(x, y)
                open_sub()
                cur_pts.append((x, y))
                ncmd += 1
                cx, cy = x, y
                lk = 0
            elif cmd in (b"c", b"s"):
                if cmd == b"c":
                    c1 = (args[0], args[1])
                    rest = args[2:]
                else:
                    c1 = (2 * cx - lcx, 2 * cy - lcy) if lk == 1 else (cx, cy)
                    rest = args
                c2, e = (rest[0], rest[1]), (rest[2], rest[3])
                ck(*c1, *c2, *e)
                open_sub()
                cur_pts += cubic_pts((cx, cy), c1, c2, e)
                ncmd += 1
                lcx, lcy = c2
                cx, cy = e
                lk = 1
            elif cmd in (b"q", b"t"):
                if cmd == b"q":
                    q = (args[0], args[1])
                    e = (args[2], args[3])
                else:
                    q = (2 * cx - lcx, 2 * cy - lcy) if lk == 2 else (cx, cy)
                    e = (args[0], args[1])
                c1 = (cx + 2 / 3 * (q[0] - cx), cy + 2 / 3 * (q[1] - cy))
                c2 = (e[0] + 2 / 3 * (q[0] - e[0]), e[1] + 2 / 3 * (q[1] - e[1]))
                ck(*c1, *c2, *e)
                open_sub()
                cur_pts += cubic_pts((cx, cy), c1, c2, e)
                ncmd += 1
                lcx, lcy = q
                cx, cy = e
                lk = 2
            else:  # arc
                rx, ry, phi, fa, fs, x2, y2 = args
                if abs(phi) > 1e6:
                    raise PErr(PDE_RANGE, p)
                pts = arc_points(cx, cy, rx, ry, phi, fa, fs, x2, y2)
                if pts:
                    open_sub()
                    cur_pts += pts
                    if rx == 0 or ry == 0:
                        ncmd += 1
                    else:
                        ncmd += arc_segments(cx, cy, rx, ry, phi, fa, fs, x2, y2)
                cx, cy = x2, y2
                lk = 0
            q0 = sep(s, p)
            if q0 < len(s) and (chr(s[q0]).isdigit() or s[q0] in b"+-."):
                p = q0
                continue
            p = q0
            break
    return rule, [(pts, cl) for pts, cl in subs], ncmd


def arc_segments(x1, y1, rx, ry, phi, fa, fs, x2, y2):
    """number of cubics the assembly uses for an arc (ceil(|dtheta| / 90 deg - 1e-9), 1..4)"""
    rx, ry = abs(rx), abs(ry)
    ph = math.radians(phi)
    c, s_ = math.cos(ph), math.sin(ph)
    dx2, dy2 = (x1 - x2) / 2, (y1 - y2) / 2
    x1p, y1p = c * dx2 + s_ * dy2, -s_ * dx2 + c * dy2
    lam = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry)
    if lam > 1:
        rx *= math.sqrt(lam)
        ry *= math.sqrt(lam)
    num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    co = math.sqrt(max(0.0, num / den)) if den > 0 else 0.0
    if fa == fs:
        co = -co
    cxp, cyp = co * rx * y1p / ry, -co * ry * x1p / rx
    ux, uy = (x1p - cxp) / rx, (y1p - cyp) / ry
    vx, vy = (-x1p - cxp) / rx, (-y1p - cyp) / ry
    dt = math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
    if not fs and dt > 0:
        dt -= 2 * math.pi
    elif fs and dt < 0:
        dt += 2 * math.pi
    n = math.ceil(abs(dt) / (math.pi / 2) - 1e-9)
    return min(4, max(1, n))


# ------------------------------------------------------------------ reference raster


def raster(subs, rule, W, H, scale=1.0, tx=0.0, ty=0.0, ss=SS):
    """coverage 0..1 of the polygons (every subpath closed) with the winding rule"""
    edges = []
    for pts, _cl in subs:
        q = [(x * scale + tx, y * scale + ty) for x, y in pts]
        if len(q) < 2:
            continue
        for i in range(len(q)):
            a, b = q[i], q[(i + 1) % len(q)]
            if a[1] != b[1]:
                edges.append((a[0], a[1], b[0], b[1]))
    cov = np.zeros((H, W))
    if not edges:
        return cov
    e = np.array(edges)
    x0, y0, x1, y1 = e[:, 0], e[:, 1], e[:, 2], e[:, 3]
    dirn = np.where(y1 > y0, 1, -1)
    ya, yb = np.minimum(y0, y1), np.maximum(y0, y1)
    xs = (np.arange(W * ss) + 0.5) / ss
    acc = np.zeros((H, W * ss))
    for r in range(H * ss):
        y = (r + 0.5) / ss
        m = (ya <= y) & (y < yb)
        if not m.any():
            continue
        xi = x0[m] + (y - y0[m]) * (x1[m] - x0[m]) / (y1[m] - y0[m])
        d = dirn[m]
        order = np.argsort(xi)
        xi, d = xi[order], d[order]
        # winding at x = sum of d over crossings left of x
        cs = np.concatenate([[0], np.cumsum(d)])
        k = np.searchsorted(xi, xs, side="right")
        w = cs[k]
        inside = (w % 2 != 0) if rule == 1 else (w != 0)
        acc[r // ss] += inside
    return acc.reshape(H, W, ss).sum(axis=2) / (ss * ss)


def bounds_of(subs):
    xs = [x for pts, _ in subs for x, _y in pts]
    ys = [y for pts, _ in subs for _x, y in pts]
    if not xs:
        return None
    return min(xs), min(ys), max(xs), max(ys)


# ------------------------------------------------------------------ cases

CASES = [
    # (name, path data, W, H, scale, tx, ty)
    ("tri_rel", "M10,10 L50,10 50,50 Z m 5 5 h 20 v 20 h -20 z", 64, 64, 1, 0, 0),
    ("evenodd", "F0 M 5 5 H 55 V 55 H 5 Z M 15 15 H 45 V 45 H 15 Z", 64, 64, 1, 0, 0),
    ("nonzero_hole", "F1 M 5 5 H 55 V 55 H 5 Z M 15 15 V 45 H 45 V 15 Z", 64, 64, 1, 0, 0),
    ("nonzero_same", "F1 M 5 5 H 55 V 55 H 5 Z M 15 15 H 45 V 45 H 15 Z", 64, 64, 1, 0, 0),
    ("cubic_s", "M10 60 C 10 0 50 0 50 30 S 90 60 90 10 L 90 70 Z", 100, 80, 1, 0, 0),
    ("quad_t", "M10 50 Q 30 0 50 50 T 90 50 Z", 100, 64, 1, 0, 0),
    ("circle_arc", "M 30 10 A 20 20 0 1 1 29.9 10 Z", 64, 64, 1, 2, 2),
    ("arc_rot", "M10 40 a25 15 30 0 1 40 0 A 10 10 0 0 0 10 40z", 64, 64, 1, 5, 0),
    ("arc_small_r", "M10 30 A 2 2 0 0 1 50 30 L 50 50 L 10 50 Z", 64, 64, 1, 0, 0),
    ("arc_flags", "M 20 32 a 12 12 0 1 0 24 0 a 12 12 0 0 0 -24 0 Z", 64, 64, 1, 0, 0),
    ("compact", "M.5.5l10-5e0 1e1.5.5 20z", 40, 40, 2000, 3000, 12000),
    ("implicit_m", "m 10 10 40 0 0 40 -40 0 z", 64, 64, 1, 0, 0),
    ("open_lines", "M 10 10 L 60 60 L 10 60 M 60 10 L 60 30 L 40 10", 64, 64, 1, 0, 0),
    ("hv_rel", "M 5 5 h 30 v 10 h -10 v 20 H 5 z", 48, 48, 1, 0, 0),
    ("scaled", "M 0 0 L 1 0 L 1 1 L 0 1 Z M 0.25 0.25 L 0.25 0.75 L 0.75 0.75 L 0.75 0.25 Z",
     64, 64, 50000, 7000, 7000),
    ("xaml_ws", "F1\r\n M 10,10\tL 50,12 L 30 , 55\nZ", 64, 64, 1, 0, 0),
    ("exp", "M 1e1 1E1 L 5.0e+1 10 L 50 5e1 L 1.0e1 50 Z", 64, 64, 1, 0, 0),
    ("z_then_l", "M 10 10 L 30 10 L 30 30 Z L 30 50 L 10 50 Z", 64, 64, 1, 0, 0),
    ("empty", "", 16, 16, 1, 0, 0),
]

ERRORS = [
    # (name, data, code, offset)
    ("no_move", "L 10 10", PDE_SYNTAX, 0),
    ("missing_num", "M 10", PDE_NUMBER, 4),
    ("num_after_z", "M 10 10 Z 5", PDE_SYNTAX, 10),
    ("range", "M 1e30 0", PDE_RANGE, 6),
    ("range_coord", "M 9e6 0 l 9e6 0", PDE_RANGE, 15),
    ("bad_flag", "M 10 10 A 5 5 0 2 0 20 20", PDE_NUMBER, 16),
    ("bad_cmd", "M 10 10 X", PDE_SYNTAX, 8),
    ("bad_rule", "F2 M 0 0", PDE_SYNTAX, 1),
    ("trailing", "M 10 10 L 10", PDE_NUMBER, 12),
    ("junk", "M 10 10 L 20 20 #", PDE_SYNTAX, 16),
    ("dot_only", "M . 5", PDE_NUMBER, 2),
    ("sign_only", "M - 5", PDE_NUMBER, 2),
    ("exp_empty", "M 1e 5", PDE_NUMBER, 3),
]


def fuzz_lines(n, seed):
    rng = random.Random(seed)
    base = [c[1] for c in CASES if c[1]]
    alphabet = "MmLlHhVvCcSsQqTtAaZzF01 ,.-+eE0123456789\t" + "xyz#()[]\x7f\xc3\xa4"
    out = []
    for i in range(n):
        k = i % 4
        if k == 0:
            s = rng.choice(base)
            s = s[:rng.randrange(len(s) + 1)]
        elif k == 1:
            s = list(rng.choice(base))
            for _ in range(rng.randrange(1, 6)):
                j = rng.randrange(len(s) + 1)
                op = rng.randrange(3)
                if op == 0 and j < len(s):
                    del s[j]
                elif op == 1:
                    s.insert(j, rng.choice(alphabet))
                elif j < len(s):
                    s[j] = rng.choice(alphabet)
            s = "".join(s)
        elif k == 2:
            s = "M" + "".join(rng.choice(alphabet) for _ in range(rng.randrange(1, 80)))
        else:
            parts = ["M %d %d" % (rng.randrange(-50, 1100), rng.randrange(-50, 1100))]
            for _ in range(rng.randrange(1, 12)):
                c = rng.choice("LlHhVvCcSsQqTtAaZz")
                na = NARGS[c.lower().encode()]
                args = []
                for j in range(na):
                    if c in "Aa" and j in (3, 4):
                        args.append(str(rng.randrange(2)))
                    else:
                        args.append(("%.*f" % (rng.randrange(4), rng.uniform(-400, 400))))
                parts.append(c + " " + " ".join(args))
            s = " ".join(parts)
        s = s.replace("\n", " ").replace("\r", " ")
        out.append(s)
    return out


def xaml_fuzz(target, out, n, seed=77):
    """PaintCraft loads n mutated XAML files and draws every custom shape; 0 = ok"""
    exe = out / target / ("PaintCraft.exe" if target == "win" else "PaintCraft")
    if n <= 0 or not exe.exists():
        return 0
    rng = random.Random(seed)
    d = out / "xamlfuzz" / target
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    fx = sorted((ROOT / "apps/PaintCraft/tests/fixtures/shapes").glob("*.xaml"))
    base = [f.read_bytes() for f in fx]
    geo = fuzz_lines(n, seed)
    for i in range(n):
        if i % 2:
            b = bytearray(rng.choice(base))
            for _ in range(rng.randrange(1, 8)):
                j = rng.randrange(len(b) + 1)
                op = rng.randrange(4)
                if op == 0 and j < len(b):
                    del b[j]
                elif op == 1:
                    b.insert(j, rng.choice(b'<>/"= &;#xMLZ0123456789.,-'))
                elif op == 2 and j < len(b):
                    b[j] = rng.randrange(256)
                else:
                    del b[j:]
        else:
            b = ('<ps:SimpleGeometryShape DisplayName="F%d &amp; &#x41;&#66;" Geometry="%s"/>'
                 % (i, geo[i].replace('"', ''))).encode("utf-8", "surrogateescape")
        (d / ("f%03d.xaml" % i)).write_bytes(bytes(b))
    lines = ["size 800 600", "key o", "key o", "cmd 795 524290"]
    for i in range(29, 29 + n):
        lines += ["cmd 795 %d" % (4 << 16 | i), "cmd 795 %d" % (5 << 16 | (i % 3)),
                  "drag 300,200 %d,%d" % (340 + i % 50, 250 + i % 40), "key enter"]
    lines += ["cmd 795 262144"]
    scr = d / "run.uis"
    scr.write_text("\n".join(lines) + "\n")
    rel = lambda p: str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")
    if target == "win":
        cmd = [str(exe), "--script", rel(scr)]
        env = dict(os.environ, PDN_SHAPES_DIR=rel(d))
    else:
        cmd = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e", "env",
               "PDN_SHAPES_DIR=" + rel(d), "./" + rel(exe), "--script", rel(scr)]
        env = None
    res = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, env=env, timeout=600)
    ok = res.returncode == 0 and "crash" not in (res.stdout + res.stderr)
    print(f"[{target}] xaml fuzz: {n} files, exit {res.returncode}: {'ok' if ok else 'FAIL'}")
    if not ok:
        print(res.stdout[-2000:], res.stderr[-2000:])
    return 0 if ok else 1


def check_target(target, out, a):
    R = Runner(target, out, name="pdcheck")
    fails = 0
    work = R.work
    for name, data, W, H, sc, tx, ty in CASES:
        (work / (name + ".txt")).write_bytes(data.encode())
        R.add("pdraster", R.rel(work / (name + ".txt")), R.rel(work / (name + ".raw")), W, H,
              int(sc * 1000) if sc < 1000 else sc, int(tx * 1000) if abs(tx) < 1000 else tx,
              int(ty * 1000) if abs(ty) < 1000 else ty, 1)
    for name, data, code, off in ERRORS:
        (work / (name + ".txt")).write_bytes(data.encode())
        R.add("pdraster", R.rel(work / (name + ".txt")), R.rel(work / (name + ".raw")), 8, 8,
              1000, 0, 0, 1)
    res = R.run()
    for (name, data, W, H, sc, tx, ty), line in zip(CASES, res):
        if not line.startswith("ok "):
            print(f"  FAIL {target} {name}: {line}")
            fails += 1
            continue
        scale = sc if sc < 1000 else sc / 1000
        txx = tx if abs(tx) < 1000 else tx / 1000
        tyy = ty if abs(ty) < 1000 else ty / 1000
        img, _ = read_raw(work / (name + ".raw"))
        txt = (work / (name + ".raw.txt")).read_text().split()
        code, off, ncmd, rule = map(int, txt[:4])
        rrule, subs, rncmd = parse(data)
        ref = raster(subs, rrule, W, H, scale, txx, tyy)
        d = np.abs(img.astype(np.float64) - ref * 255)
        msg = []
        if code != 0:
            msg.append(f"code {code} at {off}")
        if ncmd != rncmd:
            msg.append(f"ncmd {ncmd} != {rncmd}")
        if rule != rrule:
            msg.append(f"rule {rule} != {rrule}")
        if d.max() > 24 or d.mean() > 0.8:
            msg.append(f"raster max {d.max():.1f} mean {d.mean():.3f}")
        b = bounds_of(subs)
        if b is not None and ncmd:
            bb = [int(v) / 1000 for v in txt[4:8]]
            if max(abs(u - v) for u, v in zip(bb, b)) > 0.05:
                msg.append(f"bounds {bb} != {tuple(round(v, 3) for v in b)}")
        if msg:
            print(f"  FAIL {target} {name}: " + "; ".join(msg))
            fails += 1
    for (name, data, code, off), line in zip(ERRORS, res[len(CASES):]):
        txt = (work / (name + ".raw.txt"))
        if not line.startswith("ok ") or not txt.exists():
            print(f"  FAIL {target} err {name}: {line}")
            fails += 1
            continue
        c, o = map(int, txt.read_text().split()[:2])
        try:
            parse(data)
            rc, ro = 0, len(data)
        except PErr as e:
            rc, ro = e.code, e.off
        if (c, o) != (code, off) or (rc, ro) != (code, off):
            print(f"  FAIL {target} err {name}: asm ({c}, {o}) ref ({rc}, {ro}) want ({code}, {off})")
            fails += 1
    # fuzz
    lines = fuzz_lines(a.fuzz, 4242)
    fin = work / "fuzz.txt"
    fin.write_bytes("\n".join(lines).encode("utf-8", "surrogateescape") + b"\n")
    R.add("pdfuzz", R.rel(fin), R.rel(work / "fuzz.out"))
    res = R.run()
    if not res[0].startswith("ok "):
        print(f"  FAIL {target} fuzz: {res[0]}")
        return fails + 1
    got = (work / "fuzz.out").read_text().splitlines()
    if len(got) != len(lines):
        print(f"  FAIL {target} fuzz: {len(got)} results for {len(lines)} lines")
        return fails + 1
    bad = 0
    for s, g in zip(lines, got):
        c = int(g.split()[0])
        try:
            parse(s.encode("utf-8"))
            rc = 0
        except PErr as e:
            rc = e.code
        # the dash stroke / raster may still fail with ERR_LIMIT / ERR_NOMEM (not here)
        if (c == 0) != (rc == 0) or (c < 0 and c != rc and c not in (-8, -15)):
            bad += 1
            if bad <= 5:
                print(f"  FAIL {target} fuzz {s!r}: asm {g} ref {rc}")
    fails += bad
    print(f"[{target}] {len(CASES)} raster cases, {len(ERRORS)} error cases, {len(lines)} fuzz "
          f"lines: {'ok' if not fails else str(fails) + ' failures'}")
    if not a.keep and not fails:
        shutil.rmtree(work, ignore_errors=True)
    return fails


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/t4")
    ap.add_argument("--fuzz", type=int, default=2000)
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--xaml", type=int, default=40)
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    fails = 0
    for t in (["win", "linux"] if a.target == "both" else [a.target]):
        fails += check_target(t, out, a)
        fails += xaml_fuzz(t, out, a.xaml)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
