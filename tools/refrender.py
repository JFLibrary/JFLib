#!/usr/bin/env python3
"""refrender.py -- reference renderer for jfguiLib SGF1 fonts (reads the .bin, not the TTF).

Produces goldens for the assembly glyph rasterizer and text layout (DESIGN.md §2.4, §3.5, §6).
stdlib + numpy only.  Conventions (identical to the asm engine, see legacy/ASMLIB/docs/FONTS.md):

Scale      size64 = round(px * 64)  (size in 1/64 px, the glyph-cache key unit)
           sx = size64 / (64 * upem)                       -- never snapped
           sy = sx, except text fonts (flags.b0 = 0) with xH > 0 and size64 <= 24*64:
                xhpx = floor(xH * sx + 0.5); if xhpx >= 1: sy = xhpx / xH
Device     X = x_fu * sx + phase / 4,  Y = -y_fu * sy   (y down; pen origin (0,0) = glyph origin
           on the baseline; the pen x origin is an integer pixel, phase 0..3 = quarter pixels)
Mask box   left = floor(xMin*sx + phase/4)  right  = ceil(xMax*sx + phase/4)
           top  = floor(-yMax*sy)            bottom = ceil(-yMin*sy)     (record bbox, font units)
           mask w = right-left, h = bottom-top; the mask pixel (i, j) covers device
           [left+i, left+i+1) x [top+j, top+j+1) -- i.e. cache entry {left, top, w, h}.
Outlines   TrueType quadratics: two consecutive off-curve points imply an on-curve midpoint; a
           contour starting off-curve starts at the last point (if on) or the midpoint of last and
           first.  Composites: components drawn at (dx, dy) font units, recursively.
Flatten    per quadratic segment in device space: n = ceil(sqrt(0.25*|p0-2p1+p2| / tol)),
           1 <= n <= 128, tol = 0.2 px, points at t = i/n (DESIGN §2.4).
Coverage   mode 'ss16' (default, DESIGN §6): 16x16 samples per pixel at ((i+.5)/16, (j+.5)/16),
           non-zero winding (crossing counted when y0 <= ys < y1, x_cross < xs);
           value = (count*255 + 128) >> 8 with count 0..256.
           mode 'area': exact signed-area accumulation as in the asm rasterizer (font-rs
           draw_line, per-row prefix sum, value = rint(255*min(1,|acc|)), round-half-even as
           cvtps2dq).  Use 'area' for tight asm comparisons: 16x16 point sampling differs from
           exact area by up to 1/32 px per edge (about 8 levels) on fractional edges.
Pen        k = ((size64 << 18) + upem/2) // upem         (24.8 px per font unit, 16.16 fixed)
           fx(v) = (v*k + 0x8000) >> 16                   (arithmetic shift = floor)
           per glyph: pen += fx(kern(prev, cur)) [if prev]; draw at pen; pen += fx(advance)
           kern(l, r) = matrix[lclass[l]*kernR + rclass[r]] * 4 font units
           placement: q = (pen + 32) >> 6 (quarter px); x = q >> 2; phase = q & 3
           width of a string = final pen (24.8).
Text       masks composited as opaque white "mask over" (K3): d = m + (d*(255-m))/255 (exact /255).
Gamma      optional cov' = round(255 * (c/255)^(1/gamma)) applied to each glyph mask (default 1.0).

CLI:
  python tools/refrender.py glyphs legacy/ASMLIB/styles/winui/assets/font_reg.bin --px 64 [--phase 0] [--mode area]
         [--cps 20-7E] --out build/golden/reg64.sgm [--png build/golden/reg64.png]
  python tools/refrender.py text legacy/ASMLIB/styles/winui/assets/font_reg.bin --px 14 --text "Hällo" [--no-kern]
         [--out t.png] [--raw t.raw]     (prints 24.8 pens and width)
  python tools/refrender.py sheet legacy/ASMLIB/styles/winui/assets/icons.bin --px 32 --out build/icons.png
Mask dump (.sgm): header 16 B {'SGM1', u32 count, u32 size64, u8 phase, u8 mode (0 ss16,
  1 area), u16 tol*1000}; per glyph 16 B {u32 cp (0xFFFFFFFF = component only), u16 glyph,
  u16 0, i16 left, i16 top, u16 w, u16 h} followed by w*h coverage bytes, row-major, top first.
"""
import argparse
import math
import os
import struct
import sys
import zlib

import numpy as np

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(TOOLS)
ASSETS = os.path.join(ROOT, "legacy", "ASMLIB", "styles", "winui", "assets")

HDR = struct.Struct("<4sHHHhhhhhhhHHHHIIIIH14x")
HDR_FIELDS = ("magic", "version", "flags", "upem", "asc", "desc", "lineGap", "capH", "xH", "ulPos",
              "ulThick", "nGlyphs", "nRanges", "kernL", "kernR", "offRanges", "offGlyphs",
              "offOutlines", "offKern", "fallback")
DEFAULT_TOL = 0.2


class SGFFont:
    """Parsed SGF1 font."""

    def __init__(self, src):
        if isinstance(src, (bytes, bytearray)):
            self.data = bytes(src)
            self.name = "<mem>"
        else:
            with open(src, "rb") as f:
                self.data = f.read()
            self.name = os.path.basename(src)
        d = self.data
        h = dict(zip(HDR_FIELDS, HDR.unpack_from(d, 0)))
        if h["magic"] != b"SGF1":
            raise ValueError(f"{self.name}: not an SGF1 file")
        self.hdr = h
        self.upem = h["upem"]
        self.is_icon = bool(h["flags"] & 1)
        self.ranges = [struct.unpack_from("<IHH", d, h["offRanges"] + 8 * i) for i in range(h["nRanges"])]
        self.recs = [struct.unpack_from("<HhhhhHI", d, h["offGlyphs"] + 16 * i) for i in range(h["nGlyphs"])]
        self.outl = h["offOutlines"]
        self.kern = None
        if h["flags"] & 2 and h["offKern"]:
            n, L, R = h["nGlyphs"], h["kernL"], h["kernR"]
            o = h["offKern"]
            self.lclass = np.frombuffer(d, np.uint8, n, o)
            self.rclass = np.frombuffer(d, np.uint8, n, o + n)
            self.kern = np.frombuffer(d, np.int8, L * R, o + 2 * n).reshape(L, R)
        self._first = [r[0] for r in self.ranges]
        self._ccache = {}

    # -- lookup ----------------------------------------------------------------------------
    def lookup(self, cp):
        """Local glyph id of cp or None (binary search over the ranges)."""
        lo, hi = 0, len(self.ranges) - 1
        while lo <= hi:
            mid = (lo + hi) >> 1
            a, c, g = self.ranges[mid]
            if cp < a:
                hi = mid - 1
            elif cp >= a + c:
                lo = mid + 1
            else:
                return g + cp - a
        return None

    def glyph_id(self, cp):
        g = self.lookup(cp)
        return self.hdr["fallback"] if g is None else g

    def codepoints(self):
        for a, c, g in self.ranges:
            for i in range(c):
                yield a + i, g + i

    def advance(self, gid):
        return self.recs[gid][0]

    def kern_fu(self, left, right):
        if self.kern is None:
            return 0
        return int(self.kern[self.lclass[left], self.rclass[right]]) * 4

    # -- outlines --------------------------------------------------------------------------
    def components(self, gid):
        n, off = self.recs[gid][5], self.recs[gid][6]
        if not n & 0x8000:
            return []
        o = self.outl + off
        return [struct.unpack_from("<Hhh", self.data, o + 8 * i) for i in range(n & 0x7FFF)]

    def contours(self, gid, dx=0, dy=0, depth=0):
        """Resolved contours [[(x, y, on), ...], ...] in font units."""
        if depth > 16:
            raise ValueError("composite nesting too deep")
        n, off = self.recs[gid][5], self.recs[gid][6]
        if n & 0x8000:
            out = []
            for g, cx, cy in self.components(gid):
                out += self.contours(g, dx + cx, dy + cy, depth + 1)
            return out
        if n == 0:
            return []
        key = gid
        if key not in self._ccache:
            o = self.outl + off
            ends = struct.unpack_from(f"<{n}H", self.data, o)
            o += 2 * n + (2 if n & 1 else 0)
            npts = ends[-1] + 1
            raw = np.frombuffer(self.data, "<i2", 2 * npts, o).reshape(npts, 2).astype(np.int32)
            xs, on, ys = raw[:, 0] >> 1, raw[:, 0] & 1, raw[:, 1]
            cs, s = [], 0
            for e in ends:
                cs.append([(int(xs[i]), int(ys[i]), int(on[i])) for i in range(s, e + 1)])
                s = e + 1
            self._ccache[key] = cs
        cs = self._ccache[key]
        if dx or dy:
            return [[(x + dx, y + dy, o) for x, y, o in c] for c in cs]
        return [list(c) for c in cs]


# ---------------------------------------------------------------------------------------------
# scale / flatten / rasterize
# ---------------------------------------------------------------------------------------------
def size64_of(px):
    return int(round(px * 64))


def scales(font, size64):
    sx = size64 / (64.0 * font.upem)
    sy = sx
    xh = font.hdr["xH"]
    if not font.is_icon and xh > 0 and size64 <= 24 * 64:
        xhpx = math.floor(xh * sx + 0.5)
        if xhpx >= 1:
            sy = xhpx / xh
    return sx, sy


def glyph_box(font, gid, size64, phase=0):
    """(left, top, w, h) of the glyph mask in device pixels relative to (pen x, baseline)."""
    sx, sy = scales(font, size64)
    _, x0, y0, x1, y1, n, _ = font.recs[gid]
    if n == 0 or (x0 == x1 and y0 == y1):
        return 0, 0, 0, 0
    ph = phase / 4.0
    left, right = math.floor(x0 * sx + ph), math.ceil(x1 * sx + ph)
    top, bottom = math.floor(-y1 * sy), math.ceil(-y0 * sy)
    return left, top, right - left, bottom - top


def flatten(contours, sx, sy, ox, oy, tol=DEFAULT_TOL):
    """Contours (font units) -> line segments float64 array (M, 4): x0, y0, x1, y1 (device)."""
    segs = []
    for c in contours:
        if len(c) < 2:
            continue
        p = [(x * sx + ox, -y * sy + oy, on) for x, y, on in c]
        if p[0][2]:
            start, rest = p[0], p[1:] + [p[0]]
        elif p[-1][2]:
            start, rest = p[-1], p
        else:
            start = ((p[-1][0] + p[0][0]) * 0.5, (p[-1][1] + p[0][1]) * 0.5, 1)
            rest = p + [start]
        cx, cy = start[0], start[1]
        ctrl = None
        for x, y, on in rest:
            if on:
                if ctrl is None:
                    segs.append((cx, cy, x, y))
                else:
                    _quad(segs, cx, cy, ctrl[0], ctrl[1], x, y, tol)
                    ctrl = None
                cx, cy = x, y
            else:
                if ctrl is not None:
                    mx, my = (ctrl[0] + x) * 0.5, (ctrl[1] + y) * 0.5
                    _quad(segs, cx, cy, ctrl[0], ctrl[1], mx, my, tol)
                    cx, cy = mx, my
                ctrl = (x, y)
    if not segs:
        return np.zeros((0, 4))
    return np.array(segs, dtype=np.float64)


def _quad(segs, x0, y0, x1, y1, x2, y2, tol):
    m = math.hypot(x0 - 2 * x1 + x2, y0 - 2 * y1 + y2)
    n = max(1, min(128, math.ceil(math.sqrt(0.25 * m / tol)))) if m > 0 else 1
    px, py = x0, y0
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        x = u * u * x0 + 2 * u * t * x1 + t * t * x2
        y = u * u * y0 + 2 * u * t * y1 + t * t * y2
        if i == n:
            x, y = x2, y2
        segs.append((px, py, x, y))
        px, py = x, y


def raster_ss16(segs, w, h):
    """16x16 supersampled non-zero coverage (uint8 h x w)."""
    if w <= 0 or h <= 0:
        return np.zeros((max(h, 0), max(w, 0)), np.uint8)
    S = 16
    R, C = h * S, w * S
    s = segs[segs[:, 1] != segs[:, 3]] if len(segs) else segs
    if not len(s):
        return np.zeros((h, w), np.uint8)
    x0, y0, x1, y1 = s[:, 0], s[:, 1], s[:, 2], s[:, 3]
    d = np.where(y1 > y0, 1, -1)
    ya, yb = np.minimum(y0, y1), np.maximum(y0, y1)
    ys = (np.arange(R) + 0.5) / S
    # rows each segment crosses: ya <= ys < yb  ->  i in [ceil(ya*S-0.5), ceil(yb*S-0.5))
    i0 = np.clip(np.ceil(ya * S - 0.5), 0, R).astype(np.int64)
    i1 = np.clip(np.ceil(yb * S - 0.5), 0, R).astype(np.int64)
    cnt = i1 - i0
    tot = int(cnt.sum())
    if tot == 0:
        return np.zeros((h, w), np.uint8)
    seg = np.repeat(np.arange(len(s)), cnt)
    row = np.arange(tot) - np.repeat(np.cumsum(cnt) - cnt, cnt) + i0[seg]
    yy = ys[row]
    xc = x0[seg] + (yy - y0[seg]) * (x1[seg] - x0[seg]) / (y1[seg] - y0[seg])
    # first sample column with xs = (j+.5)/S > xc  ->  j = floor(xc*S + 0.5)
    k = np.clip(np.floor(xc * S + 0.5), 0, C).astype(np.int64)
    acc = np.bincount(row * (C + 1) + k, weights=d[seg], minlength=R * (C + 1))
    wind = np.cumsum(acc.reshape(R, C + 1), axis=1)[:, :C]
    inside = np.abs(wind) > 0.5
    cntpx = inside.reshape(h, S, w, S).sum(axis=(1, 3))
    return ((cntpx * 255 + 128) >> 8).astype(np.uint8)


def raster_area(segs, w, h):
    """Exact signed-area accumulation (font-rs), |acc| clamped to 1 -- mirrors the asm rasterizer."""
    if w <= 0 or h <= 0:
        return np.zeros((max(h, 0), max(w, 0)), np.uint8)
    W = w + 3
    a = [0.0] * (W * h)
    for x0, y0, x1, y1 in segs.tolist():
        if y0 == y1:
            continue
        if y0 < y1:
            dr = 1.0
        else:
            dr = -1.0
            x0, y0, x1, y1 = x1, y1, x0, y0
        dxdy = (x1 - x0) / (y1 - y0)
        x = x0
        if y0 < 0:
            x -= y0 * dxdy
        ystart = max(0, int(y0))
        yend = min(h, math.ceil(y1))
        for y in range(ystart, yend):
            ls = y * W
            dy = min(y + 1.0, y1) - max(float(y), y0)
            xn = x + dxdy * dy
            d = dy * dr
            xa, xb = (x, xn) if x < xn else (xn, x)
            xaf = math.floor(xa)
            xai = int(xaf)
            xbc = math.ceil(xb)
            xbi = int(xbc)
            if xai < 0:                        # cannot happen for glyph boxes; keep safe
                xai, xaf = 0, 0.0
            if xbi <= xai + 1:
                xmf = 0.5 * (x + xn) - xaf
                a[ls + xai] += d - d * xmf
                a[ls + xai + 1] += d * xmf
            else:
                s = 1.0 / (xb - xa)
                x0f = xa - xaf
                a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f)
                x1f = xb - xbc + 1.0
                am = 0.5 * s * x1f * x1f
                a[ls + xai] += d * a0
                if xbi == xai + 2:
                    a[ls + xai + 1] += d * (1.0 - a0 - am)
                else:
                    a1 = s * (1.5 - x0f)
                    a[ls + xai + 1] += d * (a1 - a0)
                    for xi in range(xai + 2, xbi - 1):
                        a[ls + xi] += d * s
                    a2 = a1 + (xbi - xai - 3) * s
                    a[ls + xbi - 1] += d * (1.0 - a2 - am)
                a[ls + xbi] += d * am
            x = xn
    acc = np.cumsum(np.array(a).reshape(h, W), axis=1)[:, :w]
    return np.rint(np.minimum(np.abs(acc), 1.0) * 255.0).astype(np.uint8)


def gamma_lut(gamma):
    c = np.arange(256) / 255.0
    return np.floor(255.0 * c ** (1.0 / gamma) + 0.5).astype(np.uint8)


_cache = {}


def glyph_mask(font, gid, px=None, phase=0, mode="ss16", tol=DEFAULT_TOL, gamma=1.0, size64=None):
    """-> (left, top, mask uint8[h, w]) relative to the integer pen position on the baseline."""
    if size64 is None:
        size64 = size64_of(px)
    key = (id(font), gid, size64, phase, mode, tol, gamma)
    r = _cache.get(key)
    if r is not None:
        return r
    left, top, w, h = glyph_box(font, gid, size64, phase)
    if w == 0 or h == 0:
        r = (0, 0, np.zeros((0, 0), np.uint8))
    else:
        sx, sy = scales(font, size64)
        segs = flatten(font.contours(gid), sx, sy, phase / 4.0 - left, -top, tol)
        m = raster_area(segs, w, h) if mode == "area" else raster_ss16(segs, w, h)
        if gamma != 1.0:
            m = gamma_lut(gamma)[m]
        r = (left, top, m)
    _cache[key] = r
    return r


# ---------------------------------------------------------------------------------------------
# text layout (24.8 fixed point)
# ---------------------------------------------------------------------------------------------
def fx_scale(font, size64):
    return ((size64 << 18) + font.upem // 2) // font.upem


def fu_to_fx(v, k):
    return (v * k + 0x8000) >> 16


def decode_utf8(b):
    """UTF-8 decoder with the engine's error rule: any invalid/overlong/surrogate/>U+10FFFF or
    truncated sequence yields U+FFFD and advances one byte."""
    out, i, n = [], 0, len(b)
    while i < n:
        c = b[i]
        if c < 0x80:
            out.append(c); i += 1; continue
        if 0xC2 <= c <= 0xDF:
            ln, cp, mn = 2, c & 0x1F, 0x80
        elif 0xE0 <= c <= 0xEF:
            ln, cp, mn = 3, c & 0x0F, 0x800
        elif 0xF0 <= c <= 0xF4:
            ln, cp, mn = 4, c & 0x07, 0x10000
        else:
            out.append(0xFFFD); i += 1; continue
        if i + ln > n or any((b[i + j] & 0xC0) != 0x80 for j in range(1, ln)):
            out.append(0xFFFD); i += 1; continue
        for j in range(1, ln):
            cp = (cp << 6) | (b[i + j] & 0x3F)
        if cp < mn or 0xD800 <= cp <= 0xDFFF or cp > 0x10FFFF:
            out.append(0xFFFD); i += 1; continue
        out.append(cp); i += ln
    return out


def layout(font, text, px=None, kerning=True, size64=None):
    """-> ([(gid, pen24_8), ...], width24_8).  text: str, bytes (UTF-8) or list of codepoints."""
    if size64 is None:
        size64 = size64_of(px)
    if isinstance(text, (bytes, bytearray)):
        cps = decode_utf8(text)
    elif isinstance(text, str):
        cps = [ord(c) for c in text]
    else:
        cps = list(text)
    k = fx_scale(font, size64)
    pen, prev, out = 0, None, []
    for cp in cps:
        gid = font.glyph_id(cp)
        if prev is not None and kerning:
            pen += fu_to_fx(font.kern_fu(prev, gid), k)
        out.append((gid, pen))
        pen += fu_to_fx(font.advance(gid), k)
        prev = gid
    return out, pen


def place(pen):
    q = (pen + 32) >> 6
    return q >> 2, q & 3


def render_text(font, text, px, kerning=True, mode="ss16", tol=DEFAULT_TOL, gamma=1.0):
    """-> (left, top, coverage uint8[h, w], glyphs, width24_8), coords relative to (0, baseline)."""
    size64 = size64_of(px)
    glyphs, width = layout(font, text, kerning=kerning, size64=size64)
    items = []
    for gid, pen in glyphs:
        x, ph = place(pen)
        l, t, m = glyph_mask(font, gid, phase=ph, mode=mode, tol=tol, gamma=gamma, size64=size64)
        if m.size:
            items.append((x + l, t, m))
    if not items:
        return 0, 0, np.zeros((0, 0), np.uint8), glyphs, width
    L = min(i[0] for i in items); T = min(i[1] for i in items)
    Rr = max(i[0] + i[2].shape[1] for i in items); B = max(i[1] + i[2].shape[0] for i in items)
    cov = np.zeros((B - T, Rr - L), np.int32)
    for x, y, m in items:
        sub = cov[y - T:y - T + m.shape[0], x - L:x - L + m.shape[1]]
        mm = m.astype(np.int32)
        t = sub * (255 - mm) + 128
        sub[...] = mm + ((t + (t >> 8)) >> 8)
    return L, T, cov.astype(np.uint8), glyphs, width


# ---------------------------------------------------------------------------------------------
# PNG + canvas
# ---------------------------------------------------------------------------------------------
def write_png(path, img):
    """img: uint8 (h, w) gray or (h, w, 3) RGB."""
    img = np.ascontiguousarray(img, np.uint8)
    h, w = img.shape[:2]
    ct = 0 if img.ndim == 2 else 2
    rows = img.reshape(h, -1)
    raw = np.concatenate([np.zeros((h, 1), np.uint8), rows], axis=1).tobytes()

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, ct, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    d = os.path.dirname(os.path.abspath(path))
    os.makedirs(d, exist_ok=True)
    with open(path, "wb") as f:
        f.write(png)


class Canvas:
    def __init__(self, w, h, bg=(255, 255, 255)):
        self.w, self.h = w, h
        self.px = np.empty((h, w, 3), np.float32)
        self.px[:] = bg

    def mask(self, x, y, m, color=(0, 0, 0)):
        if not m.size:
            return
        h, w = m.shape
        x0, y0, x1, y1 = max(x, 0), max(y, 0), min(x + w, self.w), min(y + h, self.h)
        if x0 >= x1 or y0 >= y1:
            return
        a = m[y0 - y:y1 - y, x0 - x:x1 - x, None].astype(np.float32) / 255.0
        d = self.px[y0:y1, x0:x1]
        d[...] = d * (1 - a) + np.array(color, np.float32) * a

    def rect(self, x, y, w, h, color):
        self.px[max(y, 0):max(y + h, 0), max(x, 0):max(x + w, 0)] = color

    def frame(self, x, y, w, h, color):
        self.rect(x, y, w, 1, color); self.rect(x, y + h - 1, w, 1, color)
        self.rect(x, y, 1, h, color); self.rect(x + w - 1, y, 1, h, color)

    def text(self, font, s, px, x, baseline, color=(0, 0, 0), kerning=True, mode="ss16"):
        """Draw s with its pen starting at integer x; returns the advance width in px (float)."""
        size64 = size64_of(px)
        glyphs, width = layout(font, s, kerning=kerning, size64=size64)
        for gid, pen in glyphs:
            gx, ph = place(pen)
            l, t, m = glyph_mask(font, gid, phase=ph, mode=mode, size64=size64)
            self.mask(x + gx + l, baseline + t, m, color)
        return width / 256.0

    def save(self, path):
        write_png(path, np.clip(np.rint(self.px), 0, 255).astype(np.uint8))


def load_font(path):
    return SGFFont(path)


# ---------------------------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------------------------
def parse_cps(spec):
    cps = set()
    for part in spec.replace(",", " ").split():
        part = part.upper().replace("U+", "").replace("0X", "")
        if "-" in part:
            a, b = part.split("-")
            cps |= set(range(int(a, 16), int(b, 16) + 1))
        else:
            cps.add(int(part, 16))
    return cps


def cmd_glyphs(a):
    f = SGFFont(a.font)
    size64 = size64_of(a.px)
    items = []
    rev = {}
    for cp, g in f.codepoints():
        rev.setdefault(g, cp)
    want = parse_cps(a.cps) if a.cps else None
    for gid in range(f.hdr["nGlyphs"]):
        cp = rev.get(gid, 0xFFFFFFFF)
        if want is not None and cp not in want:
            continue
        l, t, m = glyph_mask(f, gid, phase=a.phase, mode=a.mode, tol=a.tol, size64=size64,
                             gamma=a.gamma)
        items.append((cp, gid, l, t, m))
    if a.out:
        out = bytearray(struct.pack("<4sIIBBH", b"SGM1", len(items), size64, a.phase,
                                    1 if a.mode == "area" else 0, int(round(a.tol * 1000))))
        for cp, gid, l, t, m in items:
            h, w = m.shape
            out += struct.pack("<IHHhhHH", cp, gid, 0, l, t, w, h) + m.tobytes()
        os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
        with open(a.out, "wb") as fo:
            fo.write(out)
        print(f"{a.out}: {len(items)} masks at size64 {size64} phase {a.phase} mode {a.mode}")
    if a.png:
        cell = max(max((m.shape[1] for *_, m in items), default=1),
                   max((m.shape[0] for *_, m in items), default=1)) + 4
        cols = max(1, min(len(items), 1400 // cell))
        rows = (len(items) + cols - 1) // cols
        img = np.full((rows * cell, cols * cell), 255, np.uint8)
        for i, (cp, gid, l, t, m) in enumerate(items):
            y, x = (i // cols) * cell + 2, (i % cols) * cell + 2
            img[y:y + m.shape[0], x:x + m.shape[1]] = 255 - m
        write_png(a.png, img)
        print(f"{a.png}: sheet {cols}x{rows}")


def cmd_text(a):
    f = SGFFont(a.font)
    txt = a.text.encode("utf-8") if not a.cps else [int(c, 16) for c in a.cps.replace(",", " ").split()]
    L, T, cov, glyphs, width = render_text(f, txt, a.px, kerning=not a.no_kern, mode=a.mode,
                                           tol=a.tol, gamma=a.gamma)
    print(f"size64 {size64_of(a.px)} k {fx_scale(f, size64_of(a.px))} width24.8 {width} "
          f"({width / 256:.3f} px)")
    print("pens24.8: " + " ".join(f"{g}@{p}" for g, p in glyphs))
    print(f"coverage box left {L} top {T} w {cov.shape[1]} h {cov.shape[0]}")
    if a.out:
        write_png(a.out, 255 - cov)
    if a.raw:
        with open(a.raw, "wb") as fo:
            fo.write(struct.pack("<hhHH", L, T, cov.shape[1], cov.shape[0]) + cov.tobytes())


def cmd_sheet(a):
    f = SGFFont(a.font)
    lab = SGFFont(a.label_font)
    items = list(f.codepoints())
    cellw, cellh = max(int(a.px * 1.6), 64), int(a.px * 1.5) + 18
    cols = max(1, a.cols)
    rows = (len(items) + cols - 1) // cols
    cv = Canvas(cols * cellw + 1, rows * cellh + 1)
    for i, (cp, gid) in enumerate(items):
        x, y = (i % cols) * cellw, (i // cols) * cellh
        cv.frame(x, y, cellw + 1, cellh + 1, (215, 215, 215))
        cv.text(lab, f"{cp:04X}", 11, x + 3, y + 12, (90, 90, 90))
        base = y + 14 + int(round(a.px * (f.hdr["asc"] / f.upem))) if not f.is_icon else y + 16 + int(a.px)
        cv.text(f, [cp], a.px, x + (cellw - int(a.px)) // 2, base)
    cv.save(a.out)
    print(f"{a.out}: {len(items)} glyphs")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("glyphs", "text", "sheet"):
        p = sub.add_parser(name)
        p.add_argument("font")
        p.add_argument("--px", type=float, default=64 if name == "glyphs" else 14)
        p.add_argument("--mode", choices=("ss16", "area"), default="ss16")
        p.add_argument("--tol", type=float, default=DEFAULT_TOL)
        p.add_argument("--gamma", type=float, default=1.0)
        p.add_argument("--out")
        if name == "glyphs":
            p.add_argument("--phase", type=int, default=0, choices=range(4))
            p.add_argument("--cps", help="hex ranges, e.g. 20-7E,C4")
            p.add_argument("--png")
        elif name == "text":
            p.add_argument("--text", default="")
            p.add_argument("--cps", help="hex codepoints instead of --text")
            p.add_argument("--no-kern", action="store_true")
            p.add_argument("--raw", help="raw dump: i16 left, i16 top, u16 w, u16 h, w*h bytes")
        else:
            p.add_argument("--cols", type=int, default=16)
            p.add_argument("--label-font", default=os.path.join(ASSETS, "font_reg.bin"))
    a = ap.parse_args()
    {"glyphs": cmd_glyphs, "text": cmd_text, "sheet": cmd_sheet}[a.cmd](a)
    return 0


if __name__ == "__main__":
    sys.exit(main())
