#!/usr/bin/env python3
"""var_ref_sdk.py FONT FACE AXES GIDS TEXT OUT.json -- the reference of tools/var_check.py: runs in
the Flatpak runtime org.freedesktop.Sdk//26.08 (its python3, ctypes on libharfbuzz.so.0 and
libfreetype.so.6) and writes a variable font instance's normalized coordinates (HarfBuzz,
FreeType), every listed glyph's outline (FreeType: FT_Load_Glyph NO_SCALE points, tags, contour
ends; HarfBuzz: hb_font_draw_glyph segments) and advance (both), and hb_shape of TEXT (glyph
ids, x advances).  AXES "wght=600,wdth=80" ("" = the default instance), GIDS "all" or "1,2,3".
"""
import ctypes as C
import json
import sys

hb = C.CDLL("libharfbuzz.so.0")
ft = C.CDLL("libfreetype.so.6")

path, face_i, axes, gids, text, outp = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6]
vp = C.c_void_p


# ------------------------------------------------------------------ HarfBuzz
class HbVariation(C.Structure):
    _fields_ = [("tag", C.c_uint32), ("value", C.c_float)]


class HbInfo(C.Structure):
    _fields_ = [("codepoint", C.c_uint32), ("mask", C.c_uint32), ("cluster", C.c_uint32), ("v1", C.c_uint32), ("v2", C.c_uint32)]


class HbPos(C.Structure):
    _fields_ = [("x_advance", C.c_int32), ("y_advance", C.c_int32), ("x_offset", C.c_int32), ("y_offset", C.c_int32), ("v", C.c_uint32)]


for name, res, args in [
    ("hb_blob_create_from_file", vp, [C.c_char_p]), ("hb_face_create", vp, [vp, C.c_uint]), ("hb_font_create", vp, [vp]),
    ("hb_variation_from_string", C.c_int, [C.c_char_p, C.c_int, C.POINTER(HbVariation)]),
    ("hb_font_set_variations", None, [vp, C.POINTER(HbVariation), C.c_uint]),
    ("hb_font_get_var_coords_normalized", C.POINTER(C.c_int), [vp, C.POINTER(C.c_uint)]),
    ("hb_buffer_create", vp, []), ("hb_buffer_add_utf8", None, [vp, C.c_char_p, C.c_int, C.c_uint, C.c_int]),
    ("hb_buffer_guess_segment_properties", None, [vp]), ("hb_shape", None, [vp, vp, vp, C.c_uint]),
    ("hb_buffer_get_glyph_infos", C.POINTER(HbInfo), [vp, C.POINTER(C.c_uint)]),
    ("hb_buffer_get_glyph_positions", C.POINTER(HbPos), [vp, C.POINTER(C.c_uint)]),
    ("hb_font_get_glyph_h_advance", C.c_int32, [vp, C.c_uint32]), ("hb_draw_funcs_create", vp, []),
    ("hb_font_draw_glyph", None, [vp, C.c_uint32, vp, vp]), ("hb_face_get_glyph_count", C.c_uint, [vp]),
    ("hb_version_string", C.c_char_p, []),
]:
    f = getattr(hb, name)
    f.restype = res
    f.argtypes = args

MOVE = C.CFUNCTYPE(None, vp, vp, vp, C.c_float, C.c_float, vp)
QUAD = C.CFUNCTYPE(None, vp, vp, vp, C.c_float, C.c_float, C.c_float, C.c_float, vp)
CUBIC = C.CFUNCTYPE(None, vp, vp, vp, C.c_float, C.c_float, C.c_float, C.c_float, C.c_float, C.c_float, vp)
CLOSE = C.CFUNCTYPE(None, vp, vp, vp, vp)
segs = []
cb_move = MOVE(lambda d, dd, st, x, y, u: segs.append(["M", x, y]))
cb_line = MOVE(lambda d, dd, st, x, y, u: segs.append(["L", x, y]))
cb_quad = QUAD(lambda d, dd, st, cx, cy, x, y, u: segs.append(["Q", cx, cy, x, y]))
cb_cubic = CUBIC(lambda d, dd, st, a, b, c, e, x, y, u: segs.append(["C", a, b, c, e, x, y]))
cb_close = CLOSE(lambda d, dd, st, u: segs.append(["Z"]))
dfuncs = hb.hb_draw_funcs_create()
for fn, cb in [("move_to", cb_move), ("line_to", cb_line), ("quadratic_to", cb_quad), ("cubic_to", cb_cubic),
               ("close_path", cb_close)]:
    f = getattr(hb, f"hb_draw_funcs_set_{fn}_func")
    f.restype = None
    f.argtypes = [vp, type(cb), vp, vp]
    f(dfuncs, cb, None, None)

blob = hb.hb_blob_create_from_file(path.encode())
hface = hb.hb_face_create(blob, face_i)
font = hb.hb_font_create(hface)
items = [a for a in axes.split(",") if a]
if items:
    arr = (HbVariation * len(items))()
    for i, a in enumerate(items):
        hb.hb_variation_from_string(a.encode(), -1, C.byref(arr[i]))
    hb.hb_font_set_variations(font, arr, len(items))
n = C.c_uint(0)
pc = hb.hb_font_get_var_coords_normalized(font, C.byref(n))
hb_coords = [pc[i] for i in range(n.value)]

# ------------------------------------------------------------------ FreeType
FT_Long, FT_ULong = C.c_long, C.c_ulong


class FtGeneric(C.Structure):
    _fields_ = [("data", vp), ("finalizer", vp)]


class FtBBox(C.Structure):
    _fields_ = [("xMin", FT_Long), ("yMin", FT_Long), ("xMax", FT_Long), ("yMax", FT_Long)]


class FtFace(C.Structure):
    _fields_ = [("num_faces", FT_Long), ("face_index", FT_Long), ("face_flags", FT_Long), ("style_flags", FT_Long),
                ("num_glyphs", FT_Long), ("family_name", C.c_char_p), ("style_name", C.c_char_p),
                ("num_fixed_sizes", C.c_int), ("available_sizes", vp), ("num_charmaps", C.c_int), ("charmaps", vp),
                ("generic", FtGeneric), ("bbox", FtBBox), ("units_per_EM", C.c_ushort), ("ascender", C.c_short),
                ("descender", C.c_short), ("height", C.c_short), ("max_advance_width", C.c_short),
                ("max_advance_height", C.c_short), ("underline_position", C.c_short),
                ("underline_thickness", C.c_short), ("glyph", vp)]


class FtVector(C.Structure):
    _fields_ = [("x", FT_Long), ("y", FT_Long)]


class FtMetrics(C.Structure):
    _fields_ = [(k, FT_Long) for k in ("width", "height", "hbx", "hby", "hadv", "vbx", "vby", "vadv")]


class FtBitmap(C.Structure):
    _fields_ = [("rows", C.c_uint), ("width", C.c_uint), ("pitch", C.c_int), ("buffer", vp), ("num_grays", C.c_ushort),
                ("pixel_mode", C.c_ubyte), ("palette_mode", C.c_ubyte), ("palette", vp)]


class FtOutline(C.Structure):
    _fields_ = [("n_contours", C.c_ushort), ("n_points", C.c_ushort), ("points", C.POINTER(FtVector)),
                ("tags", C.POINTER(C.c_ubyte)), ("contours", C.POINTER(C.c_ushort)), ("flags", C.c_int)]


class FtSlot(C.Structure):
    _fields_ = [("library", vp), ("face", vp), ("next", vp), ("glyph_index", C.c_uint), ("generic", FtGeneric),
                ("metrics", FtMetrics), ("linearHoriAdvance", FT_Long), ("linearVertAdvance", FT_Long),
                ("advance", FtVector), ("format", C.c_uint), ("bitmap", FtBitmap), ("bitmap_left", C.c_int),
                ("bitmap_top", C.c_int), ("outline", FtOutline)]


class FtVarAxis(C.Structure):
    _fields_ = [("name", C.c_char_p), ("minimum", FT_Long), ("def_", FT_Long), ("maximum", FT_Long), ("tag", FT_ULong),
                ("strid", C.c_uint)]


class FtMMVar(C.Structure):
    _fields_ = [("num_axis", C.c_uint), ("num_designs", C.c_uint), ("num_namedstyles", C.c_uint),
                ("axis", C.POINTER(FtVarAxis)), ("namedstyle", vp)]


lib = vp()
assert ft.FT_Init_FreeType(C.byref(lib)) == 0
fface = C.POINTER(FtFace)()
assert ft.FT_New_Face(lib, path.encode(), C.c_long(face_i), C.byref(fface)) == 0
mm = C.POINTER(FtMMVar)()
ft_coords = []
if ft.FT_Get_MM_Var(fface, C.byref(mm)) == 0:
    na = mm.contents.num_axis
    tags = {}
    for i in range(na):
        ax = mm.contents.axis[i]
        t = ax.tag.to_bytes(4, "big").decode("latin-1")
        tags[t] = (i, ax.def_)
    des = (FT_Long * na)()
    for t, (i, d) in tags.items():
        des[i] = d
    for a in items:
        t, v = a.split("=")
        if t in tags:
            des[tags[t][0]] = int(round(float(v) * 65536))
    assert ft.FT_Set_Var_Design_Coordinates(fface, na, des) == 0
    blend = (FT_Long * na)()
    ft.FT_Get_Var_Blend_Coordinates(fface, na, blend)
    ft_coords = [blend[i] / 65536.0 for i in range(na)]

LOAD_NO_SCALE = 1
ng = hb.hb_face_get_glyph_count(hface)
glist = range(ng) if gids == "all" else [int(g) for g in gids.split(",")]
glyphs = {}
for g in glist:
    rec = {}
    if ft.FT_Load_Glyph(fface, g, LOAD_NO_SCALE) == 0:
        slot = C.cast(fface.contents.glyph, C.POINTER(FtSlot)).contents
        o = slot.outline
        pts = [[o.points[i].x, o.points[i].y, o.tags[i] & 3] for i in range(o.n_points)]
        ends = [o.contours[i] for i in range(o.n_contours)]
        rec["ft"] = {"pts": pts, "ends": ends, "adv": slot.advance.x, "lin": slot.linearHoriAdvance}
    segs.clear()
    hb.hb_font_draw_glyph(font, g, dfuncs, None)
    rec["hb"] = {"segs": [list(s) for s in segs], "adv": hb.hb_font_get_glyph_h_advance(font, g)}
    glyphs[g] = rec

buf = hb.hb_buffer_create()
tb = text.encode()
hb.hb_buffer_add_utf8(buf, tb, len(tb), 0, len(tb))
hb.hb_buffer_guess_segment_properties(buf)
hb.hb_shape(font, buf, None, 0)
infos = hb.hb_buffer_get_glyph_infos(buf, C.byref(n))
poss = hb.hb_buffer_get_glyph_positions(buf, C.byref(n))
shape = [[infos[i].codepoint, poss[i].x_advance, poss[i].x_offset, infos[i].cluster] for i in range(n.value)]
json.dump({"hb_version": hb.hb_version_string().decode(), "hb_coords": hb_coords, "ft_coords": ft_coords,
           "glyphs": glyphs, "shape": shape}, open(outp, "w"))
print("ref ok", len(glyphs), "glyphs", hb_coords, ft_coords)
