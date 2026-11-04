"""gen_pn_icons.py -- PaintCraft's own colourful icons (vector layers on a 16 x 16 grid) ->
apps/PaintCraft/src/workspace/icons/generated.rs (the tables workspace/icons/mod.rs draws; the
library icon font stays as it is).

  python tools/gen_pn_icons.py

An icon replaces the glyph of one IC_* code point wherever PaintCraft draws it in the normal
text colour (tool bar, Werkzeuge window, window toggles, menus); on a disabled row the glyph of
the font stays.  A name PN_* is a code point of PaintCraft's own (private use, no glyph in the
font, generated.rs gets the constant): these icons are drawn in every colour, in greys on a
disabled row.  Give a library code point art only when all of its uses mean the same thing
(IC_UP is also the file dialog's parent folder, IC_ADD the + of the tool options).
Layers are drawn in order:
  fill(colour, path)            path filled (non-zero)
  stroke(colour, width, path)   path stroked (round caps / joins), width in grid units
  solid(fill, edge, path)       a crisp 1 px edge: stroke width 2 in the edge colour, then the
                                fill -- the visible edge is the 1 px band outside the shape
                                (on whole grid coordinates it stays sharp at 16 px)
Colour 0 = the current text colour (dark on light, light on dark themes).
"""
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "apps" / "PaintCraft" / "src" / "workspace" / "icons" / "generated.rs"
OWN_FIRST = 0x10F200  # PN_* code points (BackupCraft and JFNetworker use 0x10F000, 0x10F100)

BLUE = (0xFF3B82F6, 0xFF1E4FA8)
LBLUE = (0xFFBFDBFE, 0xFF1E4FA8)
YELLOW = (0xFFFBBF24, 0xFF9A6A08)
RED = (0xFFEF4444, 0xFF9B1C1C)
GREEN = (0xFF22C55E, 0xFF15703A)
ORANGE = (0xFFF97316, 0xFFA8480A)
PURPLE = (0xFFA855F7, 0xFF6528C2)
PAPER = (0xFFFFFFFF, 0xFF5B6B80)
METAL = (0xFFB4BFCC, 0xFF4B5563)
WOOD = (0xFFD08A4C, 0xFF7C4A1E)
PINK = (0xFFF472B6, 0xFFA3195B)
TEAL = (0xFF14B8A6, 0xFF0D6B62)
SKIN = (0xFFF9C9A0, 0xFFB0703F)
DARK = 0xFF334155
INK = 0

K_FILL, K_STROKE = 0, 2


def f(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return s if s != "-0" else "0"


def poly(*pts):
    return "M" + " L".join(f"{f(x)} {f(y)}" for x, y in pts) + " Z"


def rect(x0, y0, x1, y1):
    return poly((x0, y0), (x1, y0), (x1, y1), (x0, y1))


def circle(cx, cy, r):
    return (f"M{f(cx - r)} {f(cy)} A{f(r)} {f(r)} 0 1 0 {f(cx + r)} {f(cy)} "
            f"A{f(r)} {f(r)} 0 1 0 {f(cx - r)} {f(cy)} Z")


def ellipse(cx, cy, rx, ry):
    return (f"M{f(cx - rx)} {f(cy)} A{f(rx)} {f(ry)} 0 1 0 {f(cx + rx)} {f(cy)} "
            f"A{f(rx)} {f(ry)} 0 1 0 {f(cx - rx)} {f(cy)} Z")


def star(cx, cy, ro, ri, n=4, rot=-90):
    pts = []
    for k in range(2 * n):
        a = math.radians(rot + k * 180 / n)
        r = ro if k % 2 == 0 else ri
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return poly(*pts)


def gear(cx, cy, ro, ri, teeth=8):
    pts = []
    for k in range(teeth * 4):
        a = math.radians(k * 360 / (teeth * 4) - 90 + 360 / (teeth * 8))
        r = ro if (k % 4) in (1, 2) else ri
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return poly(*pts)


def dashes_rect(x0, y0, x1, y1, dash=2.0, gap=1.5, t=1.0):
    """dashed outline as small filled rectangles (crisp at 16 px)"""
    out = []
    for (ax, ay, bx, by) in ((x0, y0, x1, y0), (x1, y0, x1, y1), (x1, y1, x0, y1), (x0, y1, x0, y0)):
        length = abs(bx - ax) + abs(by - ay)
        s = 0.0
        while s < length:
            e = min(s + dash, length)
            if ay == by:
                sx = ax + (s if bx > ax else -s)
                ex = ax + (e if bx > ax else -e)
                out.append(rect(min(sx, ex), ay - t / 2, max(sx, ex), ay + t / 2))
            else:
                sy = ay + (s if by > ay else -s)
                ey = ay + (e if by > ay else -e)
                out.append(rect(ax - t / 2, min(sy, ey), ax + t / 2, max(sy, ey)))
            s += dash + gap
    return " ".join(out)


def dash_ellipse(cx, cy, rx, ry, n=10, duty=0.6):
    out = []
    for k in range(n):
        a0 = 2 * math.pi * k / n
        a1 = a0 + 2 * math.pi * duty / n
        x0, y0 = cx + rx * math.cos(a0), cy + ry * math.sin(a0)
        x1, y1 = cx + rx * math.cos(a1), cy + ry * math.sin(a1)
        out.append(f"M{f(x0)} {f(y0)} A{f(rx)} {f(ry)} 0 0 1 {f(x1)} {f(y1)}")
    return " ".join(out)


def diamond(cx, cy, hw, hh):
    return poly((cx - hw, cy), (cx, cy - hh), (cx + hw, cy), (cx, cy + hh))


def plus(cx, cy, arm, half):
    return poly((cx - half, cy - arm), (cx + half, cy - arm), (cx + half, cy - half), (cx + arm, cy - half),
                (cx + arm, cy + half), (cx + half, cy + half), (cx + half, cy + arm), (cx - half, cy + arm),
                (cx - half, cy + half), (cx - arm, cy + half), (cx - arm, cy - half), (cx - half, cy - half))


def arrow(x0, y0, x1, y1, stem=1.8, head=4.4, head_len=2.8):
    """a block arrow from (x0, y0) to its tip (x1, y1) as one polygon"""
    n = math.hypot(x1 - x0, y1 - y0)
    ux, uy = (x1 - x0) / n, (y1 - y0) / n
    bx, by = x1 - ux * head_len, y1 - uy * head_len
    sx, sy = -uy * stem / 2, ux * stem / 2
    hx, hy = -uy * head / 2, ux * head / 2
    return poly((x0 + sx, y0 + sy), (bx + sx, by + sy), (bx + hx, by + hy), (x1, y1),
                (bx - hx, by - hy), (bx - sx, by - sy), (x0 - sx, y0 - sy))


def on_circle(cx, cy, r, a):
    t = math.radians(a)
    return cx + r * math.cos(t), cy + r * math.sin(t)


def arc(cx, cy, r, a0, a1):
    """an arc from angle a0 to a1 in degrees (a1 > a0: clockwise on the screen)"""
    x0, y0 = on_circle(cx, cy, r, a0)
    x1, y1 = on_circle(cx, cy, r, a1)
    return (f"M{f(x0)} {f(y0)} A{f(r)} {f(r)} 0 {1 if abs(a1 - a0) > 180 else 0} {1 if a1 > a0 else 0} "
            f"{f(x1)} {f(y1)}")


def arc_head(cx, cy, r, a, sweep, width=4.8, length=3.2):
    """the head of an arc that ends at angle a (sweep > 0: it ran clockwise)"""
    px, py = on_circle(cx, cy, r, a)
    nx, ny = math.cos(math.radians(a)), math.sin(math.radians(a))
    tx, ty = -ny * sweep, nx * sweep
    return poly((px + nx * width / 2, py + ny * width / 2), (px + tx * length, py + ty * length),
                (px - nx * width / 2, py - ny * width / 2))


def wedge(cx, cy, r, a0, a1):
    x0, y0 = on_circle(cx, cy, r, a0)
    x1, y1 = on_circle(cx, cy, r, a1)
    return f"M{f(cx)} {f(cy)} L{f(x0)} {f(y0)} A{f(r)} {f(r)} 0 0 1 {f(x1)} {f(y1)} Z"


def hue(h):
    """a pure colour of hue h (degrees)"""
    c = [abs((h / 60 + k) % 6 - 3) - 1 for k in (0, 4, 2)]
    r, g, b = [int(round(255 * min(max(v, 0), 1))) for v in c]
    return 0xFF000000 | r << 16 | g << 8 | b


def fill(c, p):
    return [(c, K_FILL, 0, p)]


def stroke(c, w, p):
    return [(c, K_STROKE, w, p)]


def solid(colours, p):
    fc, ec = colours
    return [(ec, K_STROKE, 2.0, p), (fc, K_FILL, 0, p)]


ICONS = {}

ICONS["IC_PAGE"] = (solid(PAPER, poly((3.5, 1.5), (9.5, 1.5), (12.5, 4.5), (12.5, 14.5), (3.5, 14.5)))
                    + fill(0xFFCBD5E1, poly((9.5, 1.5), (12.5, 4.5), (9.5, 4.5)))
                    + solid(YELLOW, star(11.5, 11.5, 3.3, 1.2)))
ICONS["IC_FOLDER_OPEN"] = (solid(YELLOW, poly((1.5, 3.5), (5.5, 3.5), (7, 5), (13.5, 5), (13.5, 12.5), (1.5, 12.5)))
                           + solid((0xFFFCD34D, YELLOW[1]), poly((1.5, 12.5), (4, 7.5), (15, 7.5), (12.5, 12.5))))
ICONS["IC_SAVE"] = (solid(BLUE, poly((1.5, 1.5), (12.5, 1.5), (14.5, 3.5), (14.5, 14.5), (1.5, 14.5)))
                    + fill(0xFFF1F5F9, rect(4, 2, 11, 6))
                    + fill(BLUE[1], rect(8.5, 2.8, 10, 5.2))
                    + fill(0xFFE2E8F0, rect(4, 9, 12, 14))
                    + fill(0xFF94A3B8, rect(5, 10.5, 11, 11.3) + " " + rect(5, 12.2, 11, 13)))
ICONS["IC_PRINT"] = (solid(PAPER, rect(4.5, 1.5, 11.5, 6))
                     + solid(METAL, poly((1.5, 6), (14.5, 6), (14.5, 11.5), (1.5, 11.5)))
                     + solid(PAPER, rect(4.5, 9.5, 11.5, 14.5))
                     + fill(0xFF94A3B8, rect(6, 11.5, 10, 12.2))
                     + fill(GREEN[0], rect(12, 7.5, 13.2, 8.7)))
ICONS["IC_UNDO"] = (stroke(BLUE[0], 2.2, "M5 6.5 H10 A3.6 3.6 0 0 1 10 13.7 H7.5")
                    + fill(BLUE[0], poly((0.8, 6.5), (6, 1.8), (6, 11.2))))
ICONS["IC_REDO"] = (stroke(BLUE[0], 2.2, "M11 6.5 H6 A3.6 3.6 0 0 0 6 13.7 H8.5")
                    + fill(BLUE[0], poly((15.2, 6.5), (10, 1.8), (10, 11.2))))
ICONS["IC_CUT"] = (fill(METAL[1], poly((4, 0.8), (5.6, 0.8), (10.6, 9.6), (9, 10.4)))
                   + fill(0xFF94A3B8, poly((12, 0.8), (10.4, 0.8), (5.4, 9.6), (7, 10.4)))
                   + stroke(RED[0], 1.7, circle(4.5, 12.2, 2.2) + " " + circle(11.5, 12.2, 2.2)))
ICONS["IC_COPY"] = (solid(PAPER, rect(5.5, 1.5, 13.5, 10.5))
                    + solid(LBLUE, rect(2.5, 5.5, 10.5, 14.5))
                    + fill(BLUE[0], rect(4, 8, 9, 9) + " " + rect(4, 10.3, 9, 11.3) + " " + rect(4, 12.6, 7.5, 13.6)))
ICONS["IC_PASTE"] = (solid(WOOD, rect(2.5, 3, 13.5, 14.5))
                     + solid(PAPER, rect(5, 6, 12, 14.5))
                     + fill(0xFF94A3B8, rect(6.5, 8.5, 10.5, 9.3) + " " + rect(6.5, 10.6, 10.5, 11.4))
                     + solid(METAL, rect(5.5, 1.5, 10.5, 4.5)))
ICONS["IC_CLEAR_SELECTION"] = (fill(INK, dashes_rect(1.5, 1.5, 12.5, 12.5))
                               + stroke(RED[0], 2.2, "M9.5 9.5 L14.5 14.5 M14.5 9.5 L9.5 14.5"))
ICONS["IC_GRID"] = (solid(LBLUE, rect(1.5, 1.5, 14.5, 14.5))
                    + fill(BLUE[0], rect(5.5, 2, 6.5, 14) + " " + rect(9.5, 2, 10.5, 14) + " "
                           + rect(2, 5.5, 14, 6.5) + " " + rect(2, 9.5, 14, 10.5)))
ICONS["IC_RULER"] = (solid(YELLOW, poly((0.8, 10.5), (10.5, 0.8), (15.2, 5.5), (5.5, 15.2)))
                     + fill(YELLOW[1], " ".join(
                         poly((2.8 + 2.4 * k, 8.5 - 2.4 * k), (3.6 + 2.4 * k, 7.7 - 2.4 * k),
                              (5.6 + 2.4 * k + (0.8 if k % 2 else 0), 9.7 - 2.4 * k + (0.8 if k % 2 else 0)),
                              (4.8 + 2.4 * k + (0.8 if k % 2 else 0), 10.5 - 2.4 * k + (0.8 if k % 2 else 0)))
                         for k in range(4))))
ICONS["IC_CROP"] = (fill(0xFF93C5FD, rect(5.5, 5.5, 10.5, 10.5))
                    + solid(TEAL, poly((3.5, 0.8), (5, 0.8), (5, 11), (15.2, 11), (15.2, 12.5), (3.5, 12.5)))
                    + solid(TEAL, poly((0.8, 3.5), (12.5, 3.5), (12.5, 15.2), (11, 15.2), (11, 5), (0.8, 5))))
ICONS["IC_DEV_TOOLS"] = (solid(METAL, poly((5, 2.5), (11, 2.5), (11, 6.5), (9.5, 6.5), (9.5, 4), (6.5, 4), (6.5, 6.5), (5, 6.5)))
                         + solid(RED, rect(1.5, 6, 14.5, 14.5))
                         + fill(RED[1], rect(1.5, 9, 14.5, 9.8))
                         + solid(YELLOW, rect(6.5, 8, 9.5, 11)))
ICONS["IC_HISTORY"] = (solid((0xFFFFFFFF, BLUE[1]), circle(8, 8, 6.5))
                       + stroke(BLUE[1], 0.9, "M8 2.6 V3.6 M13.4 8 H12.4 M8 13.4 V12.4 M2.6 8 H3.6")
                       + stroke(INK, 1.5, "M8 4.6 V8 L10.8 9.8")
                       + fill(BLUE[0], circle(8, 8, 1.1)))
ICONS["IC_LAYERS"] = (solid((BLUE[1], 0xFF102A60), poly((1.5, 11), (8, 7.8), (14.5, 11), (8, 14.2)))
                      + solid((BLUE[0], BLUE[1]), poly((1.5, 8), (8, 4.8), (14.5, 8), (8, 11.2)))
                      + solid((0xFF93C5FD, BLUE[1]), poly((1.5, 5), (8, 1.8), (14.5, 5), (8, 8.2))))
ICONS["IC_COLOR"] = (solid((0xFFF5DEB3, WOOD[1]), "M8 1.5 A6.5 6.5 0 1 0 8 14.5 C9.6 14.5 10 13.5 9.4 12.3 "
                                                   "C8.8 11.1 9.6 10 11 10 H12.6 A1.9 1.9 0 0 0 14.5 8.1 "
                                                   "A6.6 6.6 0 0 0 8 1.5 Z")
                     + fill(RED[0], circle(4.6, 6.2, 1.5)) + fill(YELLOW[0], circle(7.6, 4.2, 1.5))
                     + fill(GREEN[0], circle(10.9, 5.2, 1.5)) + fill(BLUE[0], circle(4.4, 10, 1.5))
                     + fill(PURPLE[0], circle(6.8, 12.2, 1.3)))
ICONS["IC_SETTINGS"] = (solid(METAL, gear(8, 8, 7, 5.3))
                        + solid((0xFFF1F5F9, METAL[1]), circle(8, 8, 2.2)))
ICONS["IC_UNKNOWN"] = (solid(BLUE, circle(8, 8, 6.5))
                       + stroke(0xFFFFFFFF, 1.8, "M5.9 6.1 A2.2 2.2 0 1 1 9.4 7.9 C8.4 8.5 8 9 8 10.2")
                       + fill(0xFFFFFFFF, circle(8, 12.4, 1.1)))
# ---- tools
ICONS["IC_SELECT_RECT"] = (fill(0xFFDBEAFE, rect(2, 3, 14, 13))
                           + fill(BLUE[1], dashes_rect(2, 3, 14, 13, 2.2, 1.4, 1.2)))
ICONS["IC_MOVE"] = fill(BLUE[0], poly((8, 0.5), (10.8, 3.5), (9, 3.5), (9, 7), (12.5, 7), (12.5, 5.2),
                                      (15.5, 8), (12.5, 10.8), (12.5, 9), (9, 9), (9, 12.5), (10.8, 12.5),
                                      (8, 15.5), (5.2, 12.5), (7, 12.5), (7, 9), (3.5, 9), (3.5, 10.8),
                                      (0.5, 8), (3.5, 5.2), (3.5, 7), (7, 7), (7, 3.5), (5.2, 3.5)))
ICONS["IC_LASSO"] = (stroke(BLUE[0], 1.4, dash_ellipse(8, 6, 6.2, 4.2, 12, 0.62))
                     + stroke(WOOD[1], 1.4, "M5.2 9.8 C4.6 11.6 6.8 12.6 5.8 15")
                     + fill(WOOD[0], circle(5.4, 9.7, 1.3)))
ICONS["IC_POINTER"] = (fill(BLUE[0], dashes_rect(6, 6, 15, 15, 1.8, 1.2, 1.1))
                       + stroke(INK, 1.0, poly((2.5, 1.5), (2.5, 12), (5.1, 9.6), (6.9, 13.6),
                                               (8.6, 12.8), (6.8, 8.9), (10.2, 8.7)))
                       + fill(0xFFFFFFFF, poly((2.5, 1.5), (2.5, 12), (5.1, 9.6), (6.9, 13.6),
                                               (8.6, 12.8), (6.8, 8.9), (10.2, 8.7))))
ICONS["IC_CIRCLE_RING"] = (fill(0xFFDBEAFE, ellipse(8, 8, 6.5, 5.2))
                           + stroke(BLUE[1], 1.2, dash_ellipse(8, 8, 6.5, 5.2, 12, 0.6)))
ICONS["IC_ZOOM"] = (stroke(INK, 2.8, "M10.4 10.4 L14.6 14.6")
                    + solid((0xFFBFDBFE, METAL[1]), circle(6.6, 6.6, 4.8))
                    + fill(0xFFFFFFFF, "M4 6.2 A2.8 2.8 0 0 1 6.2 4 L6.4 5 A1.8 1.8 0 0 0 5 6.4 Z"))
ICONS["IC_MAGIC_SELECT"] = (stroke(INK, 2.2, "M2.2 13.8 L9.2 6.8")
                            + stroke(0xFFFFFFFF, 2.2, "M8.4 7.6 L9.4 6.6")
                            + solid(YELLOW, star(11.6, 4.4, 4.0, 1.5))
                            + fill(PURPLE[0], star(4.5, 4, 1.6, 0.6)) + fill(TEAL[0], star(13.5, 11.5, 1.6, 0.6)))
ICONS["IC_TOUCH_POINTER"] = (stroke(SKIN[1], 3.0, "M5 9 V4.8 M7.4 8 V3 M9.8 8 V3.4 M12.2 8.6 V5")
                             + stroke(SKIN[0], 1.4, "M5 9 V4.8 M7.4 8 V3 M9.8 8 V3.4 M12.2 8.6 V5")
                             + solid(SKIN, "M4 8 H13 V11.5 A3 3 0 0 1 10 14.5 H7.2 A3 3 0 0 1 4.6 13 L2.2 9.2 "
                                           "A1 1 0 0 1 3.8 8.2 Z"))
ICONS["IC_DROP"] = (stroke(METAL[1], 1.2, "M3.6 5.6 A4.4 4.4 0 0 1 10.6 3")
                    + solid(METAL, poly((2, 6), (9.2, 2.8), (12.3, 9.2), (6, 13)))
                    + fill(BLUE[0], poly((9.2, 2.8), (12.3, 9.2), (13.2, 8.4)))
                    + solid(BLUE, "M13.5 9.2 C13.5 9.2 11.6 11.8 11.6 13 A1.9 1.9 0 0 0 15.4 13 "
                                  "C15.4 11.8 13.5 9.2 13.5 9.2 Z"))
ICONS["IC_CONTRAST"] = (fill(0xFF1D4ED8, rect(1.5, 1.5, 4.75, 14.5)) + fill(0xFF3B82F6, rect(4.75, 1.5, 8, 14.5))
                        + fill(0xFF93C5FD, rect(8, 1.5, 11.25, 14.5)) + fill(0xFFEFF6FF, rect(11.25, 1.5, 14.5, 14.5))
                        + stroke(0xFF1E3A8A, 1.0, rect(1.5, 1.5, 14.5, 14.5)))
ICONS["IC_BRUSH"] = (stroke(WOOD[1], 3.0, "M9.2 6.8 L14.4 1.6")
                     + stroke(WOOD[0], 1.6, "M9.2 6.8 L14.4 1.6")
                     + solid(METAL, poly((6.4, 7.6), (8.4, 5.6), (10.4, 7.6), (8.4, 9.6)))
                     + solid(BLUE, "M7.2 9.6 L6.4 8.8 C4.6 9 3.6 10 3 11.8 C2.6 13.2 2.1 14 1.2 14.8 "
                                   "C3.6 15 5.8 14.2 6.9 12.6 C7.5 11.7 7.7 10.6 7.2 9.6 Z"))
ICONS["IC_ERASER"] = (solid((0xFFE2E8F0, 0xFF64748B), poly((1.5, 10.5), (5.5, 6.5), (11, 12), (8, 15), (4, 15)))
                      + solid(PINK, poly((5.5, 6.5), (10, 2), (15, 7), (11, 12)))
                      + fill(INK, rect(8, 14.4, 15, 15.2)))
ICONS["IC_PENCIL"] = (solid(YELLOW, poly((4.2, 9.8), (11, 3), (13, 5), (6.2, 11.8)))
                      + fill(0xFFFDE68A, poly((4.8, 10.2), (11.3, 3.7), (12, 4.4), (5.5, 10.9)))
                      + solid((0xFFF5D0A0, WOOD[1]), poly((4.2, 9.8), (6.2, 11.8), (1.8, 14.2)))
                      + fill(INK, poly((1.8, 14.2), (2.6, 12.6), (3.4, 13.4)))
                      + solid(PINK, poly((11, 3), (12.2, 1.8), (14.2, 3.8), (13, 5))))
ICONS["IC_EYEDROPPER"] = (stroke(0xFF93C5FD, 1.6, "M2.2 13.8 L6.2 9.8")
                          + stroke(INK, 2.3, "M5.4 10.6 L10.2 5.8")
                          + stroke(INK, 1.6, "M8 4.6 L11.4 8")
                          + solid(RED, circle(11.9, 4.1, 2.6)))
ICONS["IC_PICTURES"] = (solid(WOOD, circle(8, 3.6, 2.4))
                        + solid(WOOD, rect(7, 5.5, 9, 9))
                        + solid(RED, rect(2.5, 9, 13.5, 12.5))
                        + fill(DARK, rect(3, 12.5, 13, 14.6)))
ICONS["IC_CRAYON"] = (solid(RED, poly((3.5, 12.5), (10.5, 5.5), (13, 8), (6, 15)))
                      + fill(RED[1], poly((10.5, 5.5), (14.8, 1.2), (13, 8)))
                      + fill(0xFFFFFFFF, poly((5.4, 10.6), (8.6, 7.4), (9.6, 8.4), (6.4, 11.6)))
                      + solid(BLUE, circle(4, 4, 2.4)))
ICONS["IC_FONT"] = (fill(INK, "M1.8 13.4 L6.4 1.5 H9.6 L14.2 13.4 H11.5 L10.4 10.4 H5.6 L4.5 13.4 Z "
                              "M6.4 8.2 H9.6 L8 3.9 Z")
                    + fill(RED[0], rect(1.5, 14.4, 14.5, 15.6)))
ICONS["IC_CURVE"] = (stroke(BLUE[0], 1.9, "M2.5 12.5 C5 1.5 10.5 14.5 13.5 3.5")
                     + fill(INK, rect(1, 11, 4, 14) + " " + rect(12, 2, 15, 5)))
ICONS["IC_SHAPES"] = (solid(RED, rect(1.5, 6.5, 8.5, 13.5))
                      + solid(BLUE, circle(10.5, 6, 4.2))
                      + solid(YELLOW, poly((8.5, 15), (12, 9), (15.5, 15))))


# ---- Ansicht, Bild, Ebenen
SLATE = 0xFF64748B  # an edge that reads on both themes
LENS = 0xFFBFDBFE
L_REST = (0xFF93C5FD, BLUE[1])  # the resting layers of the small stacks
L_GO = (YELLOW[0], YELLOW[1])  # the layer a go-to command selects
L_MOVE = (ORANGE[0], ORANGE[1])  # the layer a move command moves
AHEAD = 0xFF16A34A  # the arrow of the move commands: a green that reads on both themes
TURN = ORANGE[0]  # the arcs and arrows of the operations on the whole image
HALF_R = "M8 1.5 A6.5 6.5 0 0 1 8 14.5 Z"


def picture(x0, y0, x1, y1, frame=PAPER, sky=LENS, hill=GREEN[0], sun=None):
    """a small landscape in a frame"""
    w, h = x1 - x0, y1 - y0
    out = solid(frame, rect(x0, y0, x1, y1)) + fill(sky, rect(x0, y0, x1, y1))
    if sun is not None:
        out += fill(sun, circle(x0 + w * 0.72, y0 + h * 0.3, h * 0.16))
    return out + fill(hill, poly((x0, y1), (x0, y0 + h * 0.62), (x0 + w * 0.34, y0 + h * 0.32),
                                 (x0 + w * 0.62, y0 + h * 0.68), (x0 + w * 0.78, y0 + h * 0.52),
                                 (x1, y0 + h * 0.74), (x1, y1)))


def lens(cx, cy, r, tx, ty, handle=2.8):
    k = r * 0.72
    return (stroke(INK, handle, f"M{f(cx + k)} {f(cy + k)} L{f(tx)} {f(ty)}")
            + solid((LENS, METAL[1]), circle(cx, cy, r)))


def stack3(hi=-1, tone=L_REST):
    """three small layers on the left, number hi (0 = the top one) in its own colour"""
    out = []
    for k in (2, 1, 0):
        out += solid(tone if k == hi else L_REST, diamond(5.2, 3.5 + 4.5 * k, 4.2, 2.1))
    return out


def chevrons(*ys, up=True):
    """the go-to commands: chevrons right of the stack (one step, or two: the end of the stack)"""
    d = 2.2 if up else -2.2
    return stroke(BLUE[0], 1.8, " ".join(f"M11.4 {f(y + d)} L13.5 {f(y)} L15.6 {f(y + d)}" for y in ys))


def move_arrow(y0, y1):
    """the move commands: a block arrow right of the stack"""
    return arrow(13.5, y0, 13.5, y1, 2.0, 5.0, 3.2)


ICONS["IC_ZOOM_IN"] = lens(6.6, 6.6, 4.8, 14.6, 14.6) + fill(GREEN[1], plus(6.6, 6.6, 3, 0.85))
ICONS["IC_ZOOM_OUT"] = lens(6.6, 6.6, 4.8, 14.6, 14.6) + fill(RED[1], rect(3.6, 5.75, 9.6, 7.45))
ICONS["IC_FIT_PAGE"] = (stroke(BLUE[0], 1.5, "M1.5 5 V1.5 H5 M11 1.5 H14.5 V5 M14.5 11 V14.5 H11 M5 14.5 H1.5 V11")
                        + picture(5, 5, 11, 11, sun=YELLOW[0]))
ICONS["PN_ZOOM_SEL"] = (fill(0xFFDBEAFE, rect(1.5, 1.5, 11.5, 11.5))
                        + fill(BLUE[1], dashes_rect(1.5, 1.5, 11.5, 11.5, 2.0, 1.5, 1.0))
                        + lens(10, 10, 3, 15, 15, 2.4))
ICONS["PN_ZOOM_ACTUAL"] = (solid((0xFFDBEAFE, BLUE[1]), rect(1, 2, 15, 14))
                           + fill(0xFF1E3A8A, rect(4, 4, 6, 12) + " " + poly((2, 6.4), (4, 4), (4, 6.4)) + " "
                                  + rect(7, 5, 9, 7) + " " + rect(7, 9, 9, 11) + " "
                                  + rect(12, 4, 14, 12) + " " + poly((10, 6.4), (12, 4), (12, 6.4))))
ICONS["IC_RESIZE"] = (stroke(BLUE[0], 1.3, "M2 5.5 V2 H14 V14 H10.5")
                      + picture(2, 9, 7, 14)
                      + fill(TURN, arrow(6.5, 9.5, 12.5, 3.5, 2.0, 5.4, 3.4)))
ICONS["PN_CANVAS"] = (solid(PAPER, rect(1, 1, 15, 15))
                      + solid(BLUE, rect(6, 6, 10, 10))
                      + fill(TURN, poly((8, 1.8), (10.4, 4.2), (5.6, 4.2)) + " " + poly((8, 14.2), (10.4, 11.8), (5.6, 11.8)) + " "
                             + poly((1.8, 8), (4.2, 5.6), (4.2, 10.4)) + " " + poly((14.2, 8), (11.8, 5.6), (11.8, 10.4))))
ICONS["PN_FLIP_H"] = (fill(INK, " ".join(rect(7, y, 8, y + 2) for y in (1, 4, 7, 10, 13)))
                      + solid(BLUE, poly((5, 3), (5, 13), (1, 13)))
                      + solid(LBLUE, poly((10, 3), (10, 13), (14, 13))))
ICONS["PN_FLIP_V"] = (fill(INK, " ".join(rect(x, 7, x + 2, 8) for x in (1, 4, 7, 10, 13)))
                      + solid(BLUE, poly((3, 5), (13, 5), (13, 1)))
                      + solid(LBLUE, poly((3, 10), (13, 10), (13, 14))))
ICONS["IC_ROTATE_CW"] = (solid(LBLUE, rect(6, 6, 10, 10))
                         + stroke(TURN, 1.9, arc(8, 8, 6, 100, 345))
                         + fill(TURN, arc_head(8, 8, 6, 345, 1, 5.4, 3.8)))
ICONS["IC_ROTATE_CCW"] = (solid(LBLUE, rect(6, 6, 10, 10))
                          + stroke(TURN, 1.9, arc(8, 8, 6, 80, -165))
                          + fill(TURN, arc_head(8, 8, 6, -165, -1, 5.4, 3.8)))
ICONS["PN_ROT_180"] = (solid(LBLUE, poly((4, 9), (7, 14), (1, 14)))
                       + solid(BLUE, poly((12, 14), (15, 9), (9, 9)))
                       + stroke(TURN, 1.8, arc(8, 6, 4.6, 180, 360))
                       + fill(TURN, arc_head(8, 6, 4.6, 360, 1, 4.6, 3)))
ICONS["PN_PROFILE"] = (stroke(SLATE, 2.0, poly((8, 2), (15, 14), (1, 14)))
                       + fill(RED[0], poly((8, 2), (11.5, 8), (4.5, 8)))
                       + fill(GREEN[0], poly((4.5, 8), (8, 14), (1, 14)))
                       + fill(BLUE[0], poly((11.5, 8), (15, 14), (8, 14)))
                       + fill(0xFFFFFFFF, poly((4.5, 8), (11.5, 8), (8, 14))))
ICONS["PN_FLATTEN"] = (solid(BLUE, diamond(8, 8, 6.4, 2.8))
                       + fill(TURN, arrow(8, 0.5, 8, 4.8, 2.0, 5.4, 2.6) + " " + arrow(8, 15.5, 8, 11.2, 2.0, 5.4, 2.6)))
ICONS["PN_LAYER_NEW"] = (solid((BLUE[0], BLUE[1]), diamond(6.5, 11.5, 5.4, 2.5))
                         + solid(L_REST, diamond(6.5, 8, 5.4, 2.5))
                         + solid(GREEN, plus(12, 4, 3, 1)))
ICONS["IC_DELETE"] = (stroke(RED[1], 1.1, "M6 2.4 V1 H10 V2.4")
                      + solid(METAL, poly((4, 6), (12, 6), (11, 14), (5, 14)))
                      + fill(METAL[1], rect(6.5, 7.5, 7.5, 12.5) + " " + rect(8.5, 7.5, 9.5, 12.5))
                      + solid(RED, rect(3, 3, 13, 4)))
ICONS["IC_DOWNLOAD"] = (solid(BLUE, diamond(8, 11.6, 6.4, 3))
                        + solid(L_REST, diamond(8, 4.6, 4.8, 2.3))
                        + fill(TURN, arrow(8, 1.2, 8, 10.4, 2.0, 5.2, 3)))
ICONS["PN_LAYER_IMPORT"] = (solid(YELLOW, poly((1, 6), (4.5, 6), (6, 7.5), (11, 7.5), (11, 14), (1, 14)))
                            + solid(L_REST, diamond(10.6, 4.4, 4.4, 2.4)))
ICONS["PN_ROTZOOM"] = (lens(7.4, 9, 3.4, 14.4, 15.4, 2.4)
                       + fill(GREEN[1], plus(7.4, 9, 2, 0.6))
                       + stroke(TURN, 1.6, arc(7.4, 9, 6.4, 195, 325))
                       + fill(TURN, arc_head(7.4, 9, 6.4, 325, 1, 4.4, 3)))
ICONS["PN_GO_TOP"] = stack3(0, L_GO) + chevrons(3, 7.6)
ICONS["PN_GO_UP"] = stack3(1, L_GO) + chevrons(6.4)
ICONS["PN_GO_DOWN"] = stack3(1, L_GO) + chevrons(9.6, up=False)
ICONS["PN_GO_BOTTOM"] = stack3(2, L_GO) + chevrons(8.4, 13, up=False)
ICONS["PN_LAYER_TO_TOP"] = stack3(0, L_MOVE) + fill(AHEAD, rect(11, 1, 16, 2.4) + " " + move_arrow(15, 3.4))
ICONS["PN_LAYER_UP"] = stack3(1, L_MOVE) + fill(AHEAD, move_arrow(14.5, 1.5))
ICONS["PN_LAYER_DOWN"] = stack3(1, L_MOVE) + fill(AHEAD, move_arrow(1.5, 14.5))
ICONS["PN_LAYER_TO_BOTTOM"] = stack3(2, L_MOVE) + fill(AHEAD, rect(11, 13.6, 16, 15) + " " + move_arrow(1, 12.6))
ICONS["IC_VIEW"] = (solid((0xFFFFFFFF, SLATE), "M1 8 C3.2 4.2 5.6 3 8 3 C10.4 3 12.8 4.2 15 8 C12.8 11.8 10.4 13 8 13 C5.6 13 3.2 11.8 1 8 Z")
                    + fill(BLUE[0], circle(8, 8, 3.4))
                    + fill(0xFF1E3A8A, circle(8, 8, 1.6))
                    + fill(0xFFFFFFFF, circle(9.2, 6.8, 0.8)))
ICONS["PN_LAYER_PROPS"] = (solid((BLUE[0], BLUE[1]), diamond(6, 12.4, 5, 2.4))
                           + solid(L_REST, diamond(6, 9.4, 5, 2.4))
                           + solid(YELLOW, poly((8.5, 6.5), (12.5, 2.5), (14.5, 4.5), (10.5, 8.5)))
                           + solid((0xFFF5D0A0, WOOD[1]), poly((8.5, 6.5), (10.5, 8.5), (7, 10))))
# ---- Korrekturen
ICONS["IC_BRIGHTNESS"] = (stroke(0xFFD97706, 1.5, " ".join(
                              "M{} {} L{} {}".format(*map(f, on_circle(8, 8, 5.6, a) + on_circle(8, 8, 7.2, a)))
                              for a in range(0, 360, 45)))
                          + solid((YELLOW[0], YELLOW[1]), circle(8, 8, 3.6))
                          + fill(0xFF78350F, "M8 4.4 A3.6 3.6 0 0 1 8 11.6 Z"))
ICONS["PN_ADJ_BALANCE"] = (fill(0xFF94A3B8, rect(1, 3, 15, 4) + " " + rect(1, 8, 15, 9) + " " + rect(1, 13, 15, 14))
                           + solid(RED, circle(10.5, 3.5, 1.5))
                           + solid(GREEN, circle(5.5, 8.5, 1.5))
                           + solid(BLUE, circle(9.5, 13.5, 1.5)))
ICONS["PN_ADJ_BW_PLUS"] = (solid((0xFFFFFFFF, SLATE), circle(6, 8, 4.6))
                           + fill(0xFF1F2937, "M6 3.4 A4.6 4.6 0 0 1 6 12.6 Z")
                           + fill(RED[0], rect(12, 2, 15, 5))
                           + fill(GREEN[0], rect(12, 6.5, 15, 9.5))
                           + fill(BLUE[0], rect(12, 11, 15, 14)))
ICONS["PN_ADJ_INV_ALPHA"] = (solid((0xFFFFFFFF, SLATE), rect(2, 2, 14, 14))
                             + fill(0xFFA8B3C2, " ".join(rect(2 + 2 * i, 2 + 2 * j, 4 + 2 * i, 4 + 2 * j)
                                                         for i in range(3) for j in range(6) if (i + j) % 2 == 0))
                             + fill(PURPLE[0], rect(8, 2, 14, 14))
                             + fill(PURPLE[0], "M8 4.5 A3.5 3.5 0 0 0 8 11.5 Z")
                             + fill(0xFFFFFFFF, "M8 4.5 A3.5 3.5 0 0 1 8 11.5 Z")
                             + fill(0xFFA8B3C2, rect(8, 6, 9.5, 8) + " " + rect(9.5, 8, 11, 10)))
ICONS["PN_ADJ_AUTO"] = (picture(1, 5, 11, 15)
                        + solid(YELLOW, star(11.5, 4.5, 4.0, 1.3))
                        + fill(0xFFD97706, star(4, 2.6, 2.4, 0.7)))
ICONS["PN_ADJ_EXPOSURE"] = (solid((0xFFFDE68A, SLATE), rect(2, 2, 14, 14))
                            + fill(DARK, poly((14, 2), (14, 14), (2, 14)))
                            + fill(DARK, plus(6, 6, 3, 1))
                            + fill(0xFFFFFFFF, rect(8, 10, 13, 12)))
ICONS["PN_ADJ_INVERT"] = (solid((0xFF1D4ED8, SLATE), rect(2, 2, 14, 14))
                          + fill(0xFFFB923C, rect(8, 2, 14, 14))
                          + fill(0xFFFB923C, "M8 4 A4 4 0 0 0 8 12 Z")
                          + fill(0xFF1D4ED8, "M8 4 A4 4 0 0 1 8 12 Z"))
ICONS["PN_ADJ_HUESAT"] = (stroke(SLATE, 2.0, circle(8, 8, 6.5))
                          + [l for k in range(12) for l in fill(hue(k * 30), wedge(8, 8, 6.5, k * 30 - 106, k * 30 - 74))]
                          + fill(0xFFFFFFFF, circle(8, 8, 2.2)))
ICONS["PN_ADJ_HISHADOW"] = (solid((0xFFFEF3C7, SLATE), rect(2, 2, 14, 14))
                            + fill(DARK, rect(2, 2, 8, 14))
                            + fill(0xFFFDE68A, arrow(5, 12.5, 5, 3.5, 1.6, 4.4, 3))
                            + fill(DARK, arrow(11, 3.5, 11, 12.5, 1.6, 4.4, 3)))
ICONS["PN_ADJ_CURVES"] = (solid(PAPER, rect(2, 2, 14, 14))
                          + fill(0xFFCBD5E1, rect(6, 2, 7, 14) + " " + rect(10, 2, 11, 14) + " " + rect(2, 6, 14, 7) + " " + rect(2, 10, 14, 11))
                          + stroke(BLUE[1], 1.5, "M2.6 13.4 C7.2 13.4 8.8 2.6 13.4 2.6")
                          + fill(RED[0], circle(8, 8, 1.7)))
ICONS["PN_ADJ_LEVELS"] = (solid(PAPER, rect(2, 1, 14, 9))
                          + fill(DARK, " ".join(rect(2 + 2 * k, 9 - h, 4 + 2 * k, 9) for k, h in enumerate((2, 4, 7, 5, 3, 2))))
                          + solid((0xFF111827, SLATE), poly((3, 11.5), (5, 15), (1, 15)))
                          + solid((0xFF94A3B8, SLATE), poly((8, 11.5), (10, 15), (6, 15)))
                          + solid((0xFFFFFFFF, SLATE), poly((13, 11.5), (15, 15), (11, 15))))
ICONS["PN_ADJ_BW"] = solid((0xFFFFFFFF, SLATE), circle(8, 8, 6.5)) + fill(0xFF1F2937, HALF_R)
ICONS["PN_ADJ_SEPIA"] = picture(1, 2, 15, 14, (0xFFF5E6CF, 0xFF7C4A1E), 0xFFE6C89C, 0xFF8A5A2B, 0xFFFFF1D6)
ICONS["PN_ADJ_TEMPTINT"] = (stroke(SLATE, 4.0, "M5.5 3 V10")
                            + stroke(0xFFFFFFFF, 2.0, "M5.5 3 V10")
                            + stroke(BLUE[0], 2.0, "M5.5 3 V6.5")
                            + stroke(RED[0], 2.0, "M5.5 7 V11")
                            + solid(RED, circle(5.5, 12.5, 2.2))
                            + fill(GREEN[0], circle(12.4, 4.5, 2.5))
                            + fill(0xFFDB2777, circle(12.4, 11.5, 2.5)))
ICONS["PN_ADJ_POSTERIZE"] = (stroke(0xFF7C2D12, 2.0, "M2 14 V11 H5 V8 H8 V5 H11 V2 H14 V14 Z")
                             + fill(0xFF9A3412, rect(2, 11, 5, 14))
                             + fill(0xFFEA580C, rect(5, 8, 8, 14))
                             + fill(0xFFFB923C, rect(8, 5, 11, 14))
                             + fill(0xFFFED7AA, rect(11, 2, 14, 14)))


def main():
    own = [n for n in ICONS if n.startswith("PN_")]
    layers, icons = [], []
    for name, ls in ICONS.items():
        icons.append((name, len(layers), len(ls)))
        layers.extend(ls)
    lines = ["// icons_gen -- GENERATED by tools/gen_pn_icons.py (do not edit): PaintCraft's colourful",
             "// icons as vector layers on a 16 x 16 grid (icons/mod.rs draws them)", "",
             "use ui::*;", "",
             "/// icon table row: code point, first layer, layers",
             "#[derive(Clone, Copy)]",
             "pub(crate) struct IconRec {",
             "    pub(crate) cp: u32,",
             "    pub(crate) first: u32,",
             "    pub(crate) n: u32,",
             "}", "",
             "/// layer: argb (0 = text colour), kind (0 fill, 2 stroke), stroke width (1/16 grid unit), the",
             "/// path text",
             "#[derive(Clone, Copy)]",
             "pub(crate) struct IconLayer {",
             "    pub(crate) argb: u32,",
             "    pub(crate) kind: u16,",
             "    pub(crate) width: u16,",
             "    pub(crate) path: &'static str,",
             "}", "",
             "/// the first of PaintCraft's own code points: private use, no glyph in the icon font",
             f"pub(crate) const PN_OWN_FIRST: u32 = 0x{OWN_FIRST:X};"]
    for k, name in enumerate(own):
        lines.append(f"pub(crate) const {name}: u32 = 0x{OWN_FIRST + k:X};")
    lines += ["", "/// pn_icon_tab (pn_icon_n = len)", f"pub(crate) static PN_ICON_TAB: [IconRec; {len(icons)}] = ["]
    for name, first, n in icons:
        lines.append(f"    IconRec {{ cp: {name}, first: {first}, n: {n} }},")
    lines += ["];", "", "/// pn_layer_tab (pn_layer_n = len)", f"pub(crate) static PN_LAYER_TAB: [IconLayer; {len(layers)}] = ["]
    for (col, kind, w, path) in layers:
        lines.append(f"    IconLayer {{ argb: 0x{col:08X}, kind: {kind}, width: {int(round(w * 16))}, path: \"{path}\" }},")
    lines.append("];")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{OUT.relative_to(ROOT)}: {len(icons)} icons ({len(own)} own), {len(layers)} layers")


if __name__ == "__main__":
    main()
