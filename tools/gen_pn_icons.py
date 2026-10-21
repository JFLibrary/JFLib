"""gen_pn_icons.py -- PaintCraft's own colourful icons (vector layers on a 16 x 16 grid) ->
apps/PaintCraft/src/icons_gen.s (data for pn_icons.s; the library icon font stays as it is).

  python tools/gen_pn_icons.py

An icon replaces the glyph of one IC_* code point wherever PaintCraft draws it in the normal
text colour (tool bar, Werkzeuge window, window toggles, menus).  Layers are drawn in order:
  fill(colour, path)            path filled (non-zero)
  stroke(colour, width, path)   path stroked (round caps / joins), width in grid units
  solid(fill, edge, path)       a crisp 1 px edge: stroke width 2 in the edge colour, then the
                                fill -- the visible edge is the 1 px band outside the shape
Colour 0 = the current text colour (dark on light, light on dark themes).
"""
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT.parent / "legacy" / "apps" / "PaintCraft" / "src" / "icons_gen.s"

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


def asm_str(s):
    return s.replace("\\", "\\\\").replace('"', '\\"')


def main():
    lines = ["# icons_gen.s -- GENERATED by tools/gen_pn_icons.py (do not edit): PaintCraft's colourful",
             "# icons as vector layers on a 16 x 16 grid (pn_icons.s draws them)", "",
             "        .intel_syntax noprefix", "        .include \"jfgui.inc\"", "",
             "        .section .rodata", "        .p2align 3"]
    strings, layers, icons = [], [], []
    for name, ls in ICONS.items():
        first = len(layers)
        for (col, kind, w, path) in ls:
            idx = len(strings)
            strings.append(path)
            layers.append((col, kind, int(round(w * 16)), idx, len(path.encode())))
        icons.append((name, first, len(ls)))
    lines.append("# icon table: u32 code point, u32 first layer, u32 layers, u32 0")
    lines.append("        .globl pn_icon_tab, pn_icon_n, pn_layer_tab, pn_layer_n")
    lines.append("pn_icon_tab:")
    for name, first, n in icons:
        lines.append(f"        .long   {name}, {first}, {n}, 0")
    lines.append(f"pn_icon_n:      .long   {len(icons)}")
    lines.append("        .p2align 3")
    lines.append("# layer table (32 bytes): u32 argb (0 = text colour), u16 kind (0 fill, 2 stroke),")
    lines.append("# u16 stroke width (1/16 grid unit), u32 path length, u32 0, u64 path text, u64 0")
    lines.append("pn_layer_tab:")
    for (col, kind, w, idx, ln) in layers:
        lines.append(f"        .long   0x{col:08X}")
        lines.append(f"        .short  {kind}, {w}")
        lines.append(f"        .long   {ln}, 0")
        lines.append(f"        .quad   ip_{idx}, 0")
    lines.append(f"pn_layer_n:     .long   {len(layers)}")
    for i, s in enumerate(strings):
        lines.append(f"ip_{i}: .ascii \"{asm_str(s)}\"")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{OUT.relative_to(ROOT)}: {len(icons)} icons, {len(layers)} layers")


if __name__ == "__main__":
    main()
