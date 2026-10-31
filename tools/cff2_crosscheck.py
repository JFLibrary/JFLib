#!/usr/bin/env python3
"""cff2_crosscheck.py -- glyph outlines of (variable) CFF / CFF2 fonts against FreeType and
HarfBuzz (their shared libraries through ctypes; nothing to install where they exist, e.g. on
Fedora or inside the Freedesktop SDK flatpak).

  python3 tools/cff2_crosscheck.py fixtures
      the fonts of tools/cff2_fixtures.py (apps/fonttest/tests/cff2, cff2ref) at every
      instance of expected.txt: the generator's model against FreeType and HarfBuzz
      (outline points, advances; HarfBuzz also the named instances and axes)
  python3 tools/cff2_crosscheck.py font FILE [--face N] [--axes wght=700,..] [--glyphs 0-499]
                                   [--fonttest build/linux/fonttest]
      a real font (e.g. Fedora's /usr/share/fonts/google-noto-sans-cjk-vf-fonts/
      NotoSansCJK-VF.ttc): fonttest --outlines (JFLib's outlines and advances at that
      instance) against HarfBuzz (1/64) and FreeType for the glyphs listed (default: 0-499 and
      every 97th glyph after that); prints the largest distance to HarfBuzz

HarfBuzz draws floats (the fixtures: 1/1024 tolerance).  FreeType's CFF engine builds unhinted
outlines in whole font units (its points may lie up to 1 below; it drops a closing point that
equals the first in whole units) and reads FDSelect formats 0 and 3 only, so it cannot open
jfcff2.ttc (format 4: HarfBuzz checks that file).  tools/var_check.py does the same for
TrueType variable fonts from Windows.  Exit code 0: every glyph matched.  Inside the SDK:
  flatpak run --filesystem=REPO --command=python3 org.freedesktop.Sdk//26.08 -I \\
      REPO/tools/cff2_crosscheck.py fixtures
"""
import ctypes
import ctypes.util
import math
import subprocess
import sys
import tempfile
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent


# ------------------------------------------------------------------ HarfBuzz
class HbVariation(ctypes.Structure):
    _fields_ = [("tag", ctypes.c_uint32), ("value", ctypes.c_float)]


def tag(s):
    b = s.encode("latin-1").ljust(4)
    return (b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3]


class Hb:
    def __init__(self):
        name = ctypes.util.find_library("harfbuzz") or "libharfbuzz.so.0"
        self.h = h = ctypes.CDLL(name)
        vp = ctypes.c_void_p
        h.hb_blob_create_from_file.restype = vp
        h.hb_blob_create_from_file.argtypes = [ctypes.c_char_p]
        h.hb_face_create.restype = vp
        h.hb_face_create.argtypes = [vp, ctypes.c_uint]
        h.hb_font_create.restype = vp
        h.hb_font_create.argtypes = [vp]
        h.hb_font_set_variations.argtypes = [vp, ctypes.POINTER(HbVariation), ctypes.c_uint]
        h.hb_font_set_var_named_instance.argtypes = [vp, ctypes.c_uint]
        h.hb_draw_funcs_create.restype = vp
        h.hb_font_draw_glyph.argtypes = [vp, ctypes.c_uint, vp, vp]
        h.hb_font_get_glyph_h_advance.restype = ctypes.c_int32
        h.hb_font_get_glyph_h_advance.argtypes = [vp, ctypes.c_uint]
        h.hb_face_get_glyph_count.restype = ctypes.c_uint
        h.hb_face_get_glyph_count.argtypes = [vp]
        h.hb_font_get_var_coords_normalized.restype = ctypes.POINTER(ctypes.c_int)
        h.hb_font_get_var_coords_normalized.argtypes = [vp, ctypes.POINTER(ctypes.c_uint)]
        for f in ("move_to", "line_to", "cubic_to", "quadratic_to", "close_path"):
            getattr(h, f"hb_draw_funcs_set_{f}_func").argtypes = [vp, vp, vp, vp]
        F = ctypes.c_float
        self.MOVE = ctypes.CFUNCTYPE(None, vp, vp, vp, F, F, vp)
        self.LINE = self.MOVE
        self.QUAD = ctypes.CFUNCTYPE(None, vp, vp, vp, F, F, F, F, vp)
        self.CUBIC = ctypes.CFUNCTYPE(None, vp, vp, vp, F, F, F, F, F, F, vp)
        self.CLOSE = ctypes.CFUNCTYPE(None, vp, vp, vp, vp)
        self.cur = None
        self.out = []

        def move(_d, _dd, _s, x, y, _u):
            self.cur = [(x, y, 1)]
            self.out.append(self.cur)

        def line(_d, _dd, _s, x, y, _u):
            self.cur.append((x, y, 1))

        def quad(_d, _dd, _s, cx, cy, x, y, _u):
            self.cur += [(cx, cy, 0), (x, y, 1)]

        def cubic(_d, _dd, _s, ax, ay, bx, by, x, y, _u):
            self.cur += [(ax, ay, 2), (bx, by, 2), (x, y, 1)]

        def close(_d, _dd, _s, _u):
            pass
        self.cbs = [self.MOVE(move), self.LINE(line), self.QUAD(quad), self.CUBIC(cubic), self.CLOSE(close)]
        self.df = h.hb_draw_funcs_create()
        for f, cb in zip(("move_to", "line_to", "quadratic_to", "cubic_to", "close_path"), self.cbs):
            getattr(h, f"hb_draw_funcs_set_{f}_func")(self.df, ctypes.cast(cb, vp), None, None)

    def font(self, path, face, axes=None, named=None):
        blob = self.h.hb_blob_create_from_file(str(path).encode())
        fc = self.h.hb_face_create(blob, face)
        f = self.h.hb_font_create(fc)
        if named is not None:
            self.h.hb_font_set_var_named_instance(f, named)
        if axes:
            arr = (HbVariation * len(axes))(*[HbVariation(tag(t), v) for t, v in axes.items()])
            self.h.hb_font_set_variations(f, arr, len(axes))
        return f, fc

    def outline(self, f, g):
        self.out = []
        self.h.hb_font_draw_glyph(f, g, self.df, None)
        return [c for c in self.out if c]

    def coords(self, f):
        n = ctypes.c_uint(0)
        p = self.h.hb_font_get_var_coords_normalized(f, ctypes.byref(n))
        return [p[i] for i in range(n.value)]


# ------------------------------------------------------------------ FreeType
class FtVector(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


class FtOutline(ctypes.Structure):
    _fields_ = [("n_contours", ctypes.c_ushort), ("n_points", ctypes.c_ushort), ("points", ctypes.POINTER(FtVector)),
                ("tags", ctypes.POINTER(ctypes.c_ubyte)), ("contours", ctypes.POINTER(ctypes.c_ushort)),
                ("flags", ctypes.c_int)]


class FtBitmap(ctypes.Structure):
    _fields_ = [("rows", ctypes.c_uint), ("width", ctypes.c_uint), ("pitch", ctypes.c_int),
                ("buffer", ctypes.c_void_p), ("num_grays", ctypes.c_ushort), ("pixel_mode", ctypes.c_ubyte),
                ("palette_mode", ctypes.c_ubyte), ("palette", ctypes.c_void_p)]


class FtGlyphSlot(ctypes.Structure):
    _fields_ = [("library", ctypes.c_void_p), ("face", ctypes.c_void_p), ("next", ctypes.c_void_p),
                ("glyph_index", ctypes.c_uint), ("generic", ctypes.c_void_p * 2), ("metrics", ctypes.c_long * 8),
                ("linearHoriAdvance", ctypes.c_long), ("linearVertAdvance", ctypes.c_long), ("advance", FtVector),
                ("format", ctypes.c_uint), ("bitmap", FtBitmap), ("bitmap_left", ctypes.c_int),
                ("bitmap_top", ctypes.c_int), ("outline", FtOutline)]


class FtFace(ctypes.Structure):
    _fields_ = [("num_faces", ctypes.c_long), ("face_index", ctypes.c_long), ("face_flags", ctypes.c_long),
                ("style_flags", ctypes.c_long), ("num_glyphs", ctypes.c_long), ("family_name", ctypes.c_char_p),
                ("style_name", ctypes.c_char_p), ("num_fixed_sizes", ctypes.c_int), ("available_sizes", ctypes.c_void_p),
                ("num_charmaps", ctypes.c_int), ("charmaps", ctypes.c_void_p), ("generic", ctypes.c_void_p * 2),
                ("bbox", ctypes.c_long * 4), ("units_per_EM", ctypes.c_ushort), ("ascender", ctypes.c_short),
                ("descender", ctypes.c_short), ("height", ctypes.c_short), ("max_advance_width", ctypes.c_short),
                ("max_advance_height", ctypes.c_short), ("underline_position", ctypes.c_short),
                ("underline_thickness", ctypes.c_short), ("glyph", ctypes.POINTER(FtGlyphSlot))]


class Ft:
    def __init__(self):
        name = ctypes.util.find_library("freetype") or "libfreetype.so.6"
        self.f = f = ctypes.CDLL(name)
        self.lib = ctypes.c_void_p()
        assert f.FT_Init_FreeType(ctypes.byref(self.lib)) == 0
        f.FT_New_Face.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_long, ctypes.POINTER(ctypes.POINTER(FtFace))]
        f.FT_Set_Char_Size.argtypes = [ctypes.POINTER(FtFace), ctypes.c_long, ctypes.c_long, ctypes.c_uint, ctypes.c_uint]
        f.FT_Set_Var_Design_Coordinates.argtypes = [ctypes.POINTER(FtFace), ctypes.c_uint, ctypes.POINTER(ctypes.c_long)]
        f.FT_Load_Glyph.argtypes = [ctypes.POINTER(FtFace), ctypes.c_uint, ctypes.c_int32]

    def face(self, path, index, design=None):
        """the face at one pixel per font unit (None: FreeType cannot open it)"""
        fc = ctypes.POINTER(FtFace)()
        e = self.f.FT_New_Face(self.lib, str(path).encode(), index, ctypes.byref(fc))
        if e:
            print(f"FreeType: FT_New_Face {path} {index} -> error 0x{e:x}")
            return None
        upem = fc.contents.units_per_EM
        assert self.f.FT_Set_Char_Size(fc, 0, upem * 64, 72, 72) == 0
        if design is not None:
            arr = (ctypes.c_long * len(design))(*[int(round(v * 65536)) for v in design])
            assert self.f.FT_Set_Var_Design_Coordinates(fc, len(design), arr) == 0
        return fc

    def outline(self, fc, g):
        if self.f.FT_Load_Glyph(fc, g, 2 | 8) != 0:          # NO_HINTING | NO_BITMAP
            return None
        slot = fc.contents.glyph.contents
        o = slot.outline
        out, start = [], 0
        for k in range(o.n_contours):
            end = o.contours[k]
            c = []
            for i in range(start, end + 1):
                t = o.tags[i] & 3
                c.append((o.points[i].x / 64.0, o.points[i].y / 64.0, 1 if t & 1 else (2 if t == 2 else 0)))
            out.append(c)
            start = end + 1
        return out, slot.advance.x / 64.0


# ------------------------------------------------------------------ comparison
def norm(contours):
    """drop a closing point equal to the first, empty contours"""
    out = []
    for c in contours:
        c = list(c)
        if len(c) > 1 and c[-1][2] == 1 and abs(c[-1][0] - c[0][0]) < 1e-9 and abs(c[-1][1] - c[0][1]) < 1e-9:
            c.pop()
        if c:
            out.append(c)
    return out


def same(a, b, tol, ft=False):
    """None when the outlines match; ft: FreeType's CFF engine drops the fractions of font
    units (the unhinted outline is built in whole units), so it may lie up to 1 below, and it
    drops a closing point that equals the first in whole units"""
    a, b = norm(a), norm(b)
    if len(a) != len(b):
        return f"{len(a)} vs {len(b)} contours"
    for ca, cb in zip(a, b):
        if ft and abs(len(ca) - len(cb)) == 1:
            lg = ca if len(ca) > len(cb) else cb
            if lg[-1][2] == 1 and abs(lg[-1][0] - lg[0][0]) <= 1 and abs(lg[-1][1] - lg[0][1]) <= 1:
                lg.pop()
        if len(ca) != len(cb):
            return f"contour of {len(ca)} vs {len(cb)} points"
        for p, q in zip(ca, cb):
            if p[2] != q[2]:
                return f"point {p} vs {q}"
            for u, v in ((p[0], q[0]), (p[1], q[1])):
                if abs(u - v) > tol and not (ft and abs(u - math.floor(v)) <= tol):
                    return f"point {p} vs {q}"
    return None


def fixtures():
    sys.path.insert(0, str(TOOLS))
    import cff2_fixtures as cf
    hb, ft = Hb(), Ft()
    fails = 0
    cases = [("jfcff2.otf", 0, u) for _, u, _, _ in cf.REFS] + [("jfcff2.otf", 0, u) for u in cf.ODDS]
    cases += [("jfcff2.ttc", f, u) for f in (0, 1) for _, u, _, _ in cf.REFS]
    for name, face, user in cases:
        path = cf.OUT / name
        coords, glyphs = cf.instance(user)
        design = [user.get(a[0], a[2]) for a in cf.AXES]
        hf, _ = hb.font(path, face, user)
        if hb.coords(hf) != coords and any(coords):
            print(f"FAIL {name}:{face} {user}: HarfBuzz coords {hb.coords(hf)} vs {coords}")
            fails += 1
        fc = ft.face(path, face, design)
        if fc is None and name.endswith(".ttc"):
            print("     (expected: FreeType reads FDSelect formats 0 and 3 only; the collection uses 4)")
        elif fc is None:
            fails += 1
        for gid, (cont, adv) in enumerate(glyphs):
            want = [c for c in cont if c]
            got = hb.outline(hf, gid)
            r = same(got, want, 1 / 1024)
            ha = hb.h.hb_font_get_glyph_h_advance(hf, gid)
            if r or ha != adv:
                print(f"FAIL {name}:{face} {user} g{gid} HarfBuzz: {r or ''} adv {ha} vs {adv}")
                fails += 1
            if fc is None:
                continue
            fo = ft.outline(fc, gid)
            r = "no outline" if fo is None else same(fo[0], want, 1 / 64 + 1e-9, True)
            if r or (fo and abs(fo[1] - adv) > 0.5):
                print(f"FAIL {name}:{face} {user} g{gid} FreeType: {r or ''} adv {fo and fo[1]} vs {adv}")
                fails += 1
        print(f"ok   {name}:{face} {user or 'default'}" if not fails else f"..   {name}:{face} {user}")
    # the reference CFF fonts: their outlines are the instances' (FreeType)
    for key, user, _, _ in cf.REFS:
        _, glyphs = cf.instance(user)
        fc = ft.face(cf.REF / f"ref-{key}.otf", 0)
        for gid, (cont, adv) in enumerate(glyphs):
            fo = ft.outline(fc, gid)
            r = "no outline" if fo is None else same(fo[0], [c for c in cont if c], 1 / 64 + 1e-9, True)
            if r or fo[1] != adv:
                print(f"FAIL ref-{key} g{gid}: {r} adv {fo and fo[1]} vs {adv}")
                fails += 1
    # named instances through HarfBuzz: Condensed Black = the cb instance
    hf, _ = hb.font(cf.OUT / "jfcff2.otf", 0, named=3)
    _, glyphs = cf.instance({"wght": 900, "wdth": 75})
    for gid, (cont, adv) in enumerate(glyphs):
        r = same(hb.outline(hf, gid), [c for c in cont if c], 1 / 1024)
        if r:
            print(f"FAIL named instance 3 g{gid}: {r}")
            fails += 1
    print("fixtures:", "all match" if not fails else f"{fails} mismatches")
    return 1 if fails else 0


def parse_outlines(text):
    """fonttest --outlines records -> {gid: (adv, contours)}"""
    res, cur, cont = {}, None, None
    for ln in text.splitlines():
        p = ln.split()
        if not p:
            continue
        if p[0] == "G":
            cur = []
            res[int(p[2])] = (int(p[3]), cur)
            cont = []
        elif p[0] == "P":
            cont.append((int(p[2]) / 1024.0, int(p[3]) / 1024.0, int(p[1])))
        elif p[0] == "E":
            cur.append(cont)
            cont = []
    return res


def real_font(a):
    path, face, axes, gl, exe = a.file, a.face, {}, a.glyphs, a.fonttest
    if a.axes:
        for kv in a.axes.split(","):
            k, v = kv.split("=")
            axes[k] = float(v)
    hb, ft = Hb(), Ft()
    hf, hface = hb.font(path, face, axes)
    n = hb.h.hb_face_get_glyph_count(hface)
    if gl:
        lo, _, hi = gl.partition("-")
        gids = list(range(int(lo), min(int(hi or lo) + 1, n)))
    else:
        gids = list(range(min(500, n))) + list(range(500, n, 97))
    with tempfile.TemporaryDirectory() as d:
        lst = Path(d) / "list.txt"
        lst.write_text("".join(f"g{g}\n" for g in gids))
        outp = Path(d) / "out.txt"
        cmd = [exe, "--outlines", str(path), str(face), str(lst), str(outp)]
        if axes:
            cmd += ["--axes", ",".join(f"{k}={v:g}" for k, v in axes.items())]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode:
            print("fonttest failed:", r.stdout, r.stderr)
            return 1
        jf = parse_outlines(outp.read_text())
    # FreeType takes a value for every axis: the axes not named keep fvar's default
    design = [axes.get(t, df) for t, mn, df, mx in read_fvar(path, face)] if axes else None
    fc = ft.face(path, face, design)
    if fc is None:
        print("     FreeType cannot open it: HarfBuzz only")
    fails = worst = 0
    for g in gids:
        adv, want = jf.get(g, (None, None))
        if want is None:
            print(f"FAIL g{g}: no fonttest record")
            fails += 1
            continue
        hbo = hb.outline(hf, g)
        r = same(hbo, want, 1 / 64 + 1e-6)
        ha = hb.h.hb_font_get_glyph_h_advance(hf, g)
        fo = ft.outline(fc, g) if fc is not None else ([], 0)
        r2 = "no outline" if fo is None else (same(fo[0], want, 2 / 64 + 1e-6, True) if fc is not None else None)
        if r or r2 or ha != adv:
            fails += 1
            if fails <= 20:
                print(f"FAIL g{g}: HarfBuzz {r or 'ok'} / FreeType {r2 or 'ok'} / adv {adv} HarfBuzz {ha}")
        for c1, c2 in zip(norm(hbo), norm(want)):
            for p, q in zip(c1, c2):
                worst = max(worst, abs(p[0] - q[0]), abs(p[1] - q[1]))
    print(f"{len(gids)} glyphs, {fails} differ; largest distance to HarfBuzz {worst:.5f} units")
    return 1 if fails else 0


def read_fvar(path, face):
    d = Path(path).read_bytes()
    be16 = lambda o: int.from_bytes(d[o:o + 2], "big")
    be32 = lambda o: int.from_bytes(d[o:o + 4], "big")
    base = be32(12 + 4 * face) if d[:4] == b"ttcf" else 0
    for i in range(be16(base + 4)):
        r = base + 12 + 16 * i
        if d[r:r + 4] == b"fvar":
            o = be32(r + 8)
            ao, n, asz = be16(o + 4), be16(o + 8), be16(o + 10)
            fx = lambda v: (v - (1 << 32) if v >= 1 << 31 else v) / 65536.0
            return [(d[o + ao + k * asz:o + ao + k * asz + 4].decode("latin-1"), fx(be32(o + ao + k * asz + 4)),
                     fx(be32(o + ao + k * asz + 8)), fx(be32(o + ao + k * asz + 12))) for k in range(n)]
    return []


def main():
    import argparse
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("fixtures")
    f = sub.add_parser("font")
    f.add_argument("file")
    f.add_argument("--face", type=int, default=0)
    f.add_argument("--axes")
    f.add_argument("--glyphs")
    f.add_argument("--fonttest", default=str(ROOT / "build" / "linux" / "fonttest"))
    a = ap.parse_args()
    return fixtures() if a.cmd == "fixtures" else real_font(a)


if __name__ == "__main__":
    sys.exit(main())
