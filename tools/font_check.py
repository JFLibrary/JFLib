#!/usr/bin/env python3
"""font_check.py -- check the runtime font loader (ASMLIB/src/text/ttf.s, cff.s, colr.s,
fontdb.s) through apps/fonttest against independent references.

Windows (GDI through ctypes -- a test tool only; the library never uses system font APIs):
  * outlines: GetGlyphOutlineW(GGO_NATIVE | GGO_UNHINTED | GGO_GLYPH_INDEX) with an
    identity matrix at em size (lfHeight = -unitsPerEm, so pixels = font units); CFF fonts
    with GGO_BEZIER.  Contours are compared segment by segment (tolerance 1/64 unit for
    TrueType, 1/8 unit for CFF: GDI works in 16.16 fixed point).
  * cmap: GetGlyphIndicesW for ~200 codepoints per font, glyph ids by index as well
  * advances: GGO_METRICS gmCellIncX (unhinted)
  * kerning: GetKerningPairsW ('kern' table) vs fonttest's kern-table value
  * families: EnumFontFamiliesExW vs fontdb (--names)
Linux (WSL): fc-list family names vs fontdb (--linux).
Also: SGF1 identity (embedded fonts 0..2 vs the Selawik TTFs in styles/winui/assets: glyph
masks and advances; the icon font 3 stores Fluent UI System Icons glyphs at other code points,
so ttf2bin's own verify checks it when icons.bin is written), fuzzing
(--fuzz: truncated / bit-flipped copies below build/f/fuzz) and rendering samples
(--render: PNGs below build/f/fontcheck).

  python tools/font_check.py            outlines + cmap + advances + kerning + SGF1 + render
  python tools/font_check.py --all      everything (also --names, --fuzz, --linux)
Outputs only below build/f/.  System font files are only read.
"""
import argparse
import ctypes
import os
import random
import struct
import subprocess
import sys
import time
from ctypes import wintypes
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build" / "f"
OUT = BUILD / "fontcheck"
EXE_WIN = BUILD / "win" / "fonttest.exe"
EXE_LINUX = BUILD / "linux" / "fonttest"
WINFONTS = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts"
SELAWIK = ROOT / "styles" / "winui" / "assets"     # Selawik 1.01, the sources of font_*.bin
# the TTF each embedded font (SGF1 id) was converted from
SGF_SOURCES = [(SELAWIK / "selawk.ttf", 0), (SELAWIK / "selawksb.ttf", 1), (SELAWIK / "selawkb.ttf", 2)]
WSL_DISTRO = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")

# (file, face) -- kinds: TrueType, TTC, CFF, variable, colour, symbol, CJK
FONTS = [
    ("arial.ttf", 0), ("arialbd.ttf", 0), ("ariali.ttf", 0), ("times.ttf", 0),
    ("timesbi.ttf", 0), ("segoeui.ttf", 0), ("seguisb.ttf", 0), ("segoeuib.ttf", 0),
    ("consola.ttf", 0), ("calibri.ttf", 0), ("georgia.ttf", 0), ("verdana.ttf", 0),
    ("tahoma.ttf", 0), ("comic.ttf", 0), ("impact.ttf", 0), ("cambria.ttc", 0),
    ("cambria.ttc", 1), ("msgothic.ttc", 0), ("msgothic.ttc", 1), ("simsun.ttc", 0),
    ("malgun.ttf", 0), ("symbol.ttf", 0), ("wingding.ttf", 0), ("webdings.ttf", 0),
    ("seguisym.ttf", 0), ("seguiemj.ttf", 0), ("SegUIVar.ttf", 0), ("bahnschrift.ttf", 0),
    ("SegoeIcons.ttf", 0), ("framd.ttf", 0),
]
CFF_MAX = 8                     # CFF .otf files taken from the Fonts folder

u32 = ctypes.c_uint32


def u16be(d, o):
    return struct.unpack_from(">H", d, o)[0]


def u32be(d, o):
    return struct.unpack_from(">I", d, o)[0]


def sfnt_tables(data, face):
    base = 0
    if data[:4] == b"ttcf":
        base = u32be(data, 12 + 4 * face)
    n = u16be(data, base + 4)
    t = {}
    for i in range(n):
        o = base + 12 + 16 * i
        t[data[o:o + 4].decode("latin-1")] = (u32be(data, o + 8), u32be(data, o + 12))
    return t


# ------------------------------------------------------------------ fonttest
def fonttest(*args, target="win", check=True, timeout=600):
    exe = EXE_WIN if target == "win" else EXE_LINUX
    shot = OUT / "shot.png"
    if target == "win":
        cmd = [str(exe), "--shot", str(shot)] + [str(a) for a in args]
    else:
        cmd = ["wsl.exe", "-d", WSL_DISTRO, "--cd", str(ROOT), "-e", wsl_path(exe), "--shot",
               wsl_path(shot)] + [wsl_path(a) if isinstance(a, Path) else str(a) for a in args]
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, timeout=timeout)
    out = (r.stdout or b"").decode("utf-8", "replace") + (r.stderr or b"").decode("utf-8", "replace")
    if check and r.returncode != 0:
        raise RuntimeError(f"fonttest {' '.join(map(str, args))} -> {r.returncode}\n{out}")
    return r.returncode, out


def wsl_path(p):
    p = str(Path(p).resolve())
    if len(p) > 1 and p[1] == ":":
        return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/")
    return p.replace("\\", "/")


def build(targets):
    for t in targets:
        subprocess.run([sys.executable, str(ROOT.parent / "legacy" / "build.py"), "app", "fonttest", "--out", "build/f",
                        "--target", t], cwd=ROOT, check=True, capture_output=True)


def parse_outlines(text):
    """fonttest --outlines output -> {key: dict(gid, adv, box, contours=[[(x, y, tag)]])}"""
    res = {}
    cur = None
    pts = []
    for line in text.splitlines():
        if not line:
            continue
        if line[0] == "G":
            f = line.split()
            # "G <key words...> gid adv n xmin ymin xmax ymax": the key may contain a blank
            nums = [int(v) for v in f[-7:]]
            key = " ".join(f[1:-7])
            cur = {"gid": nums[0], "adv": nums[1], "n": nums[2], "box": nums[3:7], "contours": []}
            res[key] = cur
            pts = []
        elif line[0] == "P":
            _, tag, x, y = line.split()
            pts.append((int(x) / 1024.0, int(y) / 1024.0, int(tag)))
        elif line[0] == "E":
            cur["contours"].append(pts)
            pts = []
    return res


# ------------------------------------------------------------------ canonical segments
def mid(a, b):
    return ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0)


def ours_segments(contour):
    """Our contour (points with OLT tags) -> (start, [segments]) like the emitter walks it."""
    pts = contour
    if any(p[2] == 2 for p in pts):                   # cubic contour
        i0 = next((i for i, p in enumerate(pts) if p[2] == 1), None)
        if i0 is None:
            return None
        seq = pts[i0 + 1:] + pts[:i0 + 1]
        start = pts[i0][:2]
        segs, ctrl = [], []
        for p in seq:
            if p[2] == 1:
                if not ctrl:
                    segs.append(("L", p[:2]))
                elif len(ctrl) == 1:
                    segs.append(("Q", ctrl[0], p[:2]))
                else:
                    segs.append(("C", ctrl[0], ctrl[-1], p[:2]))
                ctrl = []
            else:
                ctrl.append(p[:2])
        return start, segs
    n = len(pts)
    if pts[0][2]:
        start, seq = pts[0][:2], pts[1:]
    elif pts[-1][2]:
        start, seq = pts[-1][:2], pts[:-1]
    else:
        start, seq = mid(pts[-1][:2], pts[0][:2]), pts
    segs, pend = [], None
    for p in seq:
        xy = p[:2]
        if p[2]:
            if pend is not None:
                segs.append(("Q", pend, xy))
                pend = None
            else:
                segs.append(("L", xy))
        else:
            if pend is not None:
                segs.append(("Q", pend, mid(pend, xy)))
            pend = xy
    if pend is not None:
        segs.append(("Q", pend, start))
    return start, segs


def normalize(start, segs):
    """drop zero-length lines, close the contour, return the list of segments (empty for a
    contour whose points all coincide: GDI drops those)"""
    pts = [p for s in segs for p in s[1:]]
    if all(abs(p[0] - start[0]) < 1e-3 and abs(p[1] - start[1]) < 1e-3 for p in pts):
        return []
    out = []
    cur = start
    for s in segs:
        end = s[-1]
        if s[0] == "L" and abs(end[0] - cur[0]) < 1e-3 and abs(end[1] - cur[1]) < 1e-3:
            continue
        out.append(s)
        cur = end
    if abs(cur[0] - start[0]) > 1e-3 or abs(cur[1] - start[1]) > 1e-3:
        out.append(("L", start))
    return out


def seg_close(a, b, tol):
    if a[0] != b[0]:
        return False
    for p, q in zip(a[1:], b[1:]):
        if abs(p[0] - q[0]) > tol or abs(p[1] - q[1]) > tol:
            return False
    return True


def contour_match(ours, gdi, tol):
    """cyclic comparison: ours may start at a different point than GDI's"""
    if len(ours) != len(gdi):
        return False
    n = len(ours)
    if n == 0:
        return True
    for r in range(n):
        if all(seg_close(ours[(r + i) % n], gdi[i], tol) for i in range(n)):
            return True
    return False


def quad_to_cubic(p0, c, p1):
    return (("C", (p0[0] + 2.0 / 3 * (c[0] - p0[0]), p0[1] + 2.0 / 3 * (c[1] - p0[1])),
             (p1[0] + 2.0 / 3 * (c[0] - p1[0]), p1[1] + 2.0 / 3 * (c[1] - p1[1])), p1))


def as_cubic(start, segs):
    out, cur = [], start
    for s in segs:
        if s[0] == "Q":
            out.append(quad_to_cubic(cur, s[1], s[2]))
        else:
            out.append(s)
        cur = s[-1]
    return out


# ------------------------------------------------------------------ GDI
class GDI:
    GGO_METRICS, GGO_NATIVE, GGO_BEZIER = 0, 2, 3
    GGO_GLYPH_INDEX, GGO_UNHINTED = 0x80, 0x100

    def __init__(self):
        self.gdi = ctypes.WinDLL("gdi32", use_last_error=True)
        g = self.gdi
        g.CreateCompatibleDC.restype = ctypes.c_void_p
        g.CreateCompatibleDC.argtypes = [ctypes.c_void_p]
        g.CreateFontIndirectW.restype = ctypes.c_void_p
        g.SelectObject.restype = ctypes.c_void_p
        g.SelectObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        g.DeleteObject.argtypes = [ctypes.c_void_p]
        g.GetGlyphOutlineW.restype = wintypes.DWORD
        g.GetGlyphOutlineW.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p,
                                       wintypes.DWORD, ctypes.c_void_p, ctypes.c_void_p]
        g.GetGlyphIndicesW.restype = wintypes.DWORD
        g.GetGlyphIndicesW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int, ctypes.c_void_p,
                                       wintypes.DWORD]
        g.GetFontData.restype = wintypes.DWORD
        g.GetFontData.argtypes = [ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                                  wintypes.DWORD]
        g.GetKerningPairsW.restype = wintypes.DWORD
        g.GetKerningPairsW.argtypes = [ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        g.AddFontResourceExW.argtypes = [ctypes.c_wchar_p, wintypes.DWORD, ctypes.c_void_p]
        g.RemoveFontResourceExW.argtypes = [ctypes.c_wchar_p, wintypes.DWORD, ctypes.c_void_p]
        self.hdc = g.CreateCompatibleDC(None)
        self.font = None
        self.mat = (ctypes.c_int32 * 4)(0x10000, 0, 0, 0x10000)   # MAT2 identity (FIXED 16.16)

    class LOGFONTW(ctypes.Structure):
        _fields_ = [("lfHeight", ctypes.c_long), ("lfWidth", ctypes.c_long),
                    ("lfEscapement", ctypes.c_long), ("lfOrientation", ctypes.c_long),
                    ("lfWeight", ctypes.c_long), ("lfItalic", ctypes.c_ubyte),
                    ("lfUnderline", ctypes.c_ubyte), ("lfStrikeOut", ctypes.c_ubyte),
                    ("lfCharSet", ctypes.c_ubyte), ("lfOutPrecision", ctypes.c_ubyte),
                    ("lfClipPrecision", ctypes.c_ubyte), ("lfQuality", ctypes.c_ubyte),
                    ("lfPitchAndFamily", ctypes.c_ubyte), ("lfFaceName", ctypes.c_wchar * 32)]

    def select(self, family, height, weight, italic, charset=1):
        if self.font:
            self.gdi.DeleteObject(self.font)
        lf = self.LOGFONTW()
        lf.lfHeight = -height
        lf.lfWeight = weight
        lf.lfItalic = 1 if italic else 0
        lf.lfCharSet = charset                          # DEFAULT_CHARSET
        lf.lfOutPrecision = 7                            # OUT_TT_ONLY_PRECIS
        lf.lfQuality = 3                                 # NONANTIALIASED (irrelevant)
        lf.lfFaceName = family[:31]
        self.font = self.gdi.CreateFontIndirectW(ctypes.byref(lf))
        self.gdi.SelectObject(self.hdc, self.font)

    def table(self, tag, n):
        buf = ctypes.create_string_buffer(n)
        t = struct.unpack("<I", tag.encode("latin-1"))[0]
        r = self.gdi.GetFontData(self.hdc, t, 0, buf, n)
        return buf.raw[:r] if r != 0xFFFFFFFF else None

    def glyph_indices(self, cps):
        s = "".join(chr(c) for c in cps)
        out = (ctypes.c_uint16 * len(cps))()
        self.gdi.GetGlyphIndicesW(self.hdc, s, len(cps), out, 1)    # GGI_MARK_NONEXISTING_GLYPHS
        return list(out)

    def outline(self, gid, cubic=False):
        fmt = (self.GGO_BEZIER if cubic else self.GGO_NATIVE) | self.GGO_GLYPH_INDEX | self.GGO_UNHINTED
        gm = (ctypes.c_byte * 20)()
        n = self.gdi.GetGlyphOutlineW(self.hdc, gid, fmt, gm, 0, None, self.mat)
        if n == 0xFFFFFFFF:
            return None, None
        buf = ctypes.create_string_buffer(max(n, 1))
        if n:
            self.gdi.GetGlyphOutlineW(self.hdc, gid, fmt, gm, n, buf, self.mat)
        adv = struct.unpack_from("<h", bytes(gm), 16)[0]
        return self.parse(buf.raw[:n]), adv

    def advance(self, gid):
        gm = (ctypes.c_byte * 20)()
        n = self.gdi.GetGlyphOutlineW(self.hdc, gid, self.GGO_METRICS | self.GGO_GLYPH_INDEX |
                                      self.GGO_UNHINTED, gm, 0, None, self.mat)
        if n == 0xFFFFFFFF:
            return None
        return struct.unpack_from("<h", bytes(gm), 16)[0]

    @staticmethod
    def parse(buf):
        def fx(o):
            fract, val = struct.unpack_from("<Hh", buf, o)
            return val + fract / 65536.0
        contours, off = [], 0
        while off + 16 <= len(buf):
            cb = struct.unpack_from("<I", buf, off)[0]
            start = (fx(off + 8), fx(off + 12))
            pos, end = off + 16, off + cb
            segs = []
            while pos < end:
                wtype, cpfx = struct.unpack_from("<HH", buf, pos)
                pos += 4
                pts = [(fx(pos + 8 * i), fx(pos + 8 * i + 4)) for i in range(cpfx)]
                pos += 8 * cpfx
                if wtype == 1:
                    segs += [("L", p) for p in pts]
                elif wtype == 2:
                    for i in range(cpfx - 1):
                        e = pts[i + 1] if i == cpfx - 2 else mid(pts[i], pts[i + 1])
                        segs.append(("Q", pts[i], e))
                elif wtype == 3:
                    for i in range(0, cpfx - 2, 3):
                        segs.append(("C", pts[i], pts[i + 1], pts[i + 2]))
            contours.append((start, segs))
            off = end
        return contours

    def kerning_pairs(self):
        n = self.gdi.GetKerningPairsW(self.hdc, 0, None)
        if not n:
            return []
        arr = (ctypes.c_byte * (8 * n))()
        n = self.gdi.GetKerningPairsW(self.hdc, n, arr)
        raw = bytes(arr)
        return [struct.unpack_from("<HHi", raw, 8 * i) for i in range(n)]

    def add_private(self, path):
        return self.gdi.AddFontResourceExW(str(path), 0x10, None)    # FR_PRIVATE


# ------------------------------------------------------------------ checks
CANDIDATES = (list(range(0x20, 0x7F)) + list(range(0xA0, 0x250)) + list(range(0x370, 0x3D0)) +
              list(range(0x400, 0x460)) + list(range(0x2000, 0x2070)) + list(range(0x20A0, 0x20C0)) +
              list(range(0x2190, 0x2200)) + list(range(0x2200, 0x2300)) + list(range(0x2500, 0x2600)) +
              list(range(0x3000, 0x3040)) + list(range(0x3040, 0x30FF)) + list(range(0x4E00, 0x4E80)) +
              list(range(0xAC00, 0xAC40)) + list(range(0xE700, 0xE800)) + list(range(0xF020, 0xF100)))


def dump_info(path, face):
    _, out = fonttest("--dump", path, face)
    info = {}
    for line in out.splitlines():
        if ":" in line and not line.startswith("font"):
            k, v = line.split(":", 1)
            info[k.strip()] = v.strip()
        f = line.split()
        if f and f[0] == "upem":
            info["upem"], info["glyphs"], info["weight"] = int(f[1]), int(f[3]), int(f[5])
            info["flags"] = int(f[9], 16)
    return info


def check_font(gdi, path, face, stats, verbose):
    info = dump_info(path, face)
    upem, weight, flags = info["upem"], info["weight"], info["flags"]
    italic = bool(flags & 1)
    cff = bool(flags & 16)
    family = info.get("family", "")
    data = Path(path).read_bytes()
    tabs = sfnt_tables(data, face)
    ho, hl = tabs["head"]
    head = data[ho:ho + 54]
    gdi.add_private(path)
    chosen = None
    # variable fonts: GDI names the default instance after its named instance ("... Text")
    for fam in (family, info.get("typo family", ""), family + " Text"):
        for w in (weight, 400, 700):
            for it in (italic, not italic):
                for cs in (1, 2, 0):
                    gdi.select(fam, upem, w, it, cs)
                    h = gdi.table("head", 54)
                    if h and h[:12] == head[:12] and h[28:54] == head[28:54]:
                        chosen = (fam, w, it)
                        break
                if chosen:
                    break
            if chosen:
                break
        if chosen:
            break
    name = f"{Path(path).name}:{face} ({family})"
    if not chosen:
        print(f"  SKIP {name}: GDI does not select this face")
        stats["skipped"] += 1
        return
    # codepoints: ~200 mapped ones spread over the candidate ranges
    idx = gdi.glyph_indices(CANDIDATES)
    mapped = [(c, g) for c, g in zip(CANDIDATES, idx) if g != 0xFFFF]
    if flags & 64:                                       # symbol cmap: GDI maps U+0020.. itself
        mapped = [(c, g) for c, g in mapped]
    step = max(1, len(mapped) // 200)
    cps = mapped[::step][:220]
    nglyphs = info["glyphs"]
    gids = sorted(set(list(range(0, min(nglyphs, 40))) +
                      [random.Random(face * 7 + nglyphs).randrange(nglyphs) for _ in range(40)]))
    lst = OUT / "list.txt"
    lst.write_text("".join(f"u {c}\n" for c, _ in cps) + "".join(f"g{g}\n" for g in gids))
    res_path = OUT / "outlines.txt"
    fonttest("--outlines", path, face, lst, res_path)
    ours = parse_outlines(res_path.read_text(encoding="utf-8"))
    tol = 0.125 if cff else 1.0 / 64
    bad_map = bad_adv = bad_ol = n_ol = 0
    examples = []
    items = [(f"u {c}", g) for c, g in cps] + [(f"g{g}", g) for g in gids]
    for key, gdi_gid in items:
        rec = ours.get(key)
        if rec is None:
            bad_ol += 1
            examples.append(f"{key}: missing in fonttest output")
            continue
        if rec["gid"] != gdi_gid:
            bad_map += 1
            if len(examples) < 8:
                examples.append(f"{key}: glyph ours {rec['gid']} GDI {gdi_gid}")
            continue
        g_contours, g_adv = gdi.outline(gdi_gid, cubic=cff)
        if g_contours is None:
            continue
        if g_adv is not None and g_adv != rec["adv"]:
            bad_adv += 1
            if len(examples) < 8:
                examples.append(f"{key}: advance ours {rec['adv']} GDI {g_adv}")
        n_ol += 1
        mine = []
        for c in rec["contours"]:
            r = ours_segments(c)
            if r is not None and normalize(*r):
                mine.append(r)
        g_contours = [c for c in g_contours if normalize(*c)]      # GDI drops point contours
        ok = len(mine) == len(g_contours)
        if ok:
            for (s0, sg), (t0, tg) in zip(mine, g_contours):
                a = normalize(s0, sg)
                b = normalize(t0, tg)
                if cff:
                    a = as_cubic(s0, a) if any(s[0] == "Q" for s in a) else a
                if not contour_match(a, b, tol):
                    ok = False
                    break
        if not ok:
            bad_ol += 1
            if len(examples) < 8:
                examples.append(f"{key} (glyph {gdi_gid}): outline differs "
                                f"({len(mine)} vs {len(g_contours)} contours)")
    # kerning ('kern' table pairs as GDI reports them)
    bad_kern = n_kern = 0
    pairs = gdi.kerning_pairs()
    if pairs:
        sample = pairs[:: max(1, len(pairs) // 300)][:300]
        klist = OUT / "kern.txt"
        klist.write_text("".join(f"{a} {b}\n" for a, b, _ in sample))
        kres = OUT / "kern_out.txt"
        fonttest("--kern", path, face, klist, kres)
        rows = [list(map(int, l.split())) for l in kres.read_text().splitlines() if l.strip()]
        for (a, b, v), row in zip(sample, rows):
            n_kern += 1
            if row[5] != v and row[4] != v:
                bad_kern += 1
                if len(examples) < 10:
                    examples.append(f"kern U+{a:04X} U+{b:04X}: ours table {row[5]} gpos {row[4]} GDI {v}")
    status = "OK  " if not (bad_map or bad_adv or bad_ol or bad_kern) else "FAIL"
    print(f"  {status} {name}: {len(cps)} cps, {n_ol} outlines, {n_kern} kern pairs; "
          f"mismatches: cmap {bad_map}, adv {bad_adv}, outline {bad_ol}, kern {bad_kern}"
          + (" [CFF]" if cff else ""))
    if status == "FAIL" or verbose:
        for e in examples[:10]:
            print("       ", e)
    stats["fonts"] += 1
    stats["outlines"] += n_ol
    stats["bad"] += bad_map + bad_adv + bad_ol + bad_kern
    stats["failed_fonts"] += status == "FAIL"


def windows_checks(args):
    gdi = GDI()
    fonts = [(WINFONTS / f, i) for f, i in FONTS if (WINFONTS / f).exists()]
    cff = [p for p in sorted(WINFONTS.glob("*.otf")) if p.read_bytes()[:4] == b"OTTO"]
    fonts += [(p, 0) for p in cff[:CFF_MAX]]
    if args.fonts:
        want = [w.lower() for w in args.fonts.split(",")]
        fonts = [(p, i) for p, i in fonts if p.name.lower() in want or f"{p.name.lower()}:{i}" in want]
    stats = dict(fonts=0, outlines=0, bad=0, skipped=0, failed_fonts=0)
    print(f"outlines / cmap / advances / kerning vs GDI ({len(fonts)} fonts)")
    for p, i in fonts:
        try:
            check_font(gdi, p, i, stats, args.verbose)
        except Exception as e:                      # keep going; report
            print(f"  FAIL {p.name}:{i}: {e}")
            stats["failed_fonts"] += 1
    print(f"  => {stats['fonts']} fonts, {stats['outlines']} outlines compared, "
          f"{stats['bad']} mismatches, {stats['failed_fonts']} fonts failed, {stats['skipped']} skipped")
    return stats["failed_fonts"] == 0


def origin_fixed(src, out_dir):
    """-> (path, n): src, or a copy below out_dir whose glyf xMin equals the hmtx lsb in the n
    glyphs where they differ.  ttf2bin keeps the origin at x = 0, the run-time loader shifts the
    outline by lsb - xMin like GDI / FreeType: a glyph with a stale bbox (Selawik 1.01: U+201E,
    xMin 360, lsb 39) differs by that shift only."""
    data = bytearray(src.read_bytes())
    t = sfnt_tables(data, 0)
    if "glyf" not in t:
        return src, 0
    long_loca = u16be(data, t["head"][0] + 50)
    nglyphs = u16be(data, t["maxp"][0] + 4)
    nhm = u16be(data, t["hhea"][0] + 34)
    loca, glyf, hmtx = t["loca"][0], t["glyf"][0], t["hmtx"][0]
    n = 0
    for g in range(nglyphs):
        if long_loca:
            a, b = u32be(data, loca + 4 * g), u32be(data, loca + 4 * g + 4)
        else:
            a, b = 2 * u16be(data, loca + 2 * g), 2 * u16be(data, loca + 2 * g + 2)
        if a == b:
            continue
        lsb = data[hmtx + 4 * g + 2:hmtx + 4 * g + 4] if g < nhm else \
            data[hmtx + 4 * nhm + 2 * (g - nhm):hmtx + 4 * nhm + 2 * (g - nhm) + 2]
        if data[glyf + a + 2:glyf + a + 4] != lsb:
            data[glyf + a + 2:glyf + a + 4] = lsb
            n += 1
    if not n:
        return src, 0
    out = out_dir / f"{src.stem}_origin{src.suffix}"
    out.write_bytes(bytes(data))
    return out, n


def sgf_check():
    print("SGF1 identity: embedded fonts vs the TTF they were made from, loaded at run time (glyph masks)")
    ok = True
    for src, sgf in SGF_SOURCES:
        if not src.exists():
            print(f"  skip {src.name} (missing)")
            continue
        p, n = origin_fixed(src, OUT)
        rc, out = fonttest("--cmpsgf", p, 0, sgf, check=False)
        line = out.strip().splitlines()[-1] if out.strip() else "?"
        fixed = f" ({n} glyph origins fixed)" if n else ""
        print(f"  {'OK  ' if rc == 0 else 'FAIL'} {src.name} vs font {sgf}{fixed}: {line}")
        ok &= rc == 0
    return ok


def render_samples():
    print("render samples ->", OUT)
    samples = [("segoeui.ttf", 0, 16, "Größe AVATAR Wolf fi 123 ÄÖÜ"), ("times.ttf", 0, 36, "AVATAR Tokyo ΩΣ"),
               ("seguiemj.ttf", 0, 40, "😀🎨🦊🌈 ❤️"), ("msgothic.ttc", 0, 24, "日本語 テキスト"),
               ("bahnschrift.ttf", 0, 28, "Bahnschrift 2024"), ("symbol.ttf", 0, 28, "abgdpS")]
    cff = [p for p in sorted(WINFONTS.glob("*.otf")) if p.read_bytes()[:4] == b"OTTO"]
    if cff:
        samples.append((cff[0].name, 0, 32, "CFF Outlines 0123"))
    for f, face, px, text in samples:
        p = WINFONTS / f
        if p.exists():
            out = OUT / f"render_{Path(f).stem}.png"
            rc, _ = fonttest("--render", p, face, px, text, out, check=False)
            print(f"  {'OK  ' if rc == 0 else 'FAIL'} {out.name}")
    for style in (1, 2, 3, 4):
        out = OUT / f"render_style{style}.png"
        rc, _ = fonttest("--render", WINFONTS / "arial.ttf", 0, 32, "Synthetic AVATAR fi", out,
                         "--style", style, check=False)
        print(f"  {'OK  ' if rc == 0 else 'FAIL'} {out.name}")


def fuzz(args):
    targets = ["win"] + (["linux"] if args.linux or args.all else [])
    print(f"fuzz ({', '.join(targets)}): truncated and bit-flipped copies must not crash")
    d = BUILD / "fuzz"
    d.mkdir(parents=True, exist_ok=True)
    rng = random.Random(1234)
    seeds = ["arial.ttf", "segoeui.ttf", "cambria.ttc", "seguiemj.ttf", "SegUIVar.ttf", "symbol.ttf"]
    seeds += [p.name for p in sorted(WINFONTS.glob("*.otf")) if p.read_bytes()[:4] == b"OTTO"][:2]
    n = crashes = 0
    for s in seeds:
        src = WINFONTS / s
        if not src.exists():
            continue
        data = src.read_bytes()
        for k in range(args.fuzz_n):
            b = bytearray(data)
            mode = k % 4
            if mode == 0:
                b = b[:rng.randrange(12, len(b))]
            elif mode == 1:
                for _ in range(rng.randrange(1, 64)):
                    b[rng.randrange(len(b))] ^= 1 << rng.randrange(8)
            elif mode == 2:                           # hit the table directory and headers
                for _ in range(rng.randrange(1, 16)):
                    b[rng.randrange(min(len(b), 4096))] = rng.randrange(256)
            else:
                for _ in range(rng.randrange(1, 8)):
                    o = rng.randrange(len(b) - 4)
                    b[o:o + 4] = rng.choice([b"\xff\xff\xff\xff", b"\x00\x00\x00\x00", b"\x7f\xff\xff\xff",
                                             b"\x80\x00\x00\x00"])
            f = d / f"{Path(s).stem}_{k}{Path(s).suffix}"
            f.write_bytes(bytes(b))
            n += 1
            bad = False
            for t in targets:
                rc, out = fonttest("--fuzz", f, target=t, check=False, timeout=120)
                if rc != 0:
                    bad = True
                    print(f"  CRASH [{t}] rc={rc:#x} {f.name}: {out.strip()[-200:]}")
            if bad:
                crashes += 1
            else:
                f.unlink()
    print(f"  => {n} mutated files, {crashes} crashes")
    return crashes == 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--fonts", help="comma separated file names (file.ttf or file.ttc:1)")
    ap.add_argument("--names", action="store_true", help="families: EnumFontFamiliesExW vs fontdb")
    ap.add_argument("--fuzz", action="store_true")
    ap.add_argument("--fuzz-n", type=int, default=24, help="mutations per seed font")
    ap.add_argument("--linux", action="store_true", help="fc-list vs fontdb in WSL")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--no-outlines", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if not args.no_build:
        build(["win"] + (["linux"] if args.linux or args.all else []))
    ok = True
    if not args.no_outlines:
        ok &= windows_checks(args)
        ok &= sgf_check()
        render_samples()
        ok &= colr_check()
        ok &= cbdt_check()
    if args.names or args.all:
        ok &= names_check()
    if args.fuzz or args.all:
        ok &= fuzz(args)
    if args.linux or args.all:
        ok &= linux_check()
    print("font_check:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def gdi_families():
    """TrueType / OpenType family names as GDI enumerates them (no '@' vertical names)"""
    gdi32 = ctypes.WinDLL("gdi32")
    user32 = ctypes.WinDLL("user32")
    user32.GetDC.restype = ctypes.c_void_p
    user32.GetDC.argtypes = [ctypes.c_void_p]
    hdc = user32.GetDC(None)
    fams = {}
    PROC = ctypes.WINFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD,
                              wintypes.LPARAM)

    def cb(elf, ntm, ftype, lp):
        name = ctypes.wstring_at(elf + 28, 32).split("\0", 1)[0]
        ntm_flags = ctypes.c_uint32.from_address(ntm + 60).value
        if (ftype & 4) or (ntm_flags & 0x60000):        # TRUETYPE_FONTTYPE, NTM_*_OPENTYPE
            if not name.startswith("@"):
                fams[name.lower()] = name
        return 1
    lf = GDI.LOGFONTW()
    lf.lfCharSet = 1
    gdi32.EnumFontFamiliesExW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, PROC, wintypes.LPARAM,
                                          wintypes.DWORD]
    proc = PROC(cb)
    gdi32.EnumFontFamiliesExW(hdc, ctypes.byref(lf), proc, 0, 0)
    return fams


def parse_scan(text):
    fams, faces = {}, []
    cur = None
    for line in text.splitlines():
        if line.startswith("F "):
            cur = line[2:]
            fams[cur.lower()] = cur
        elif line.startswith("  ") and cur is not None:
            head, style, full, path = None, None, None, None
            parts = line.strip().split(" | ")
            f = parts[0].split(" ", 3)
            faces.append(dict(family=cur, weight=int(f[0]), face=int(f[1]), flags=int(f[2], 16),
                              style=f[3] if len(f) > 3 else "", full=parts[1], tfamily=parts[2],
                              path=parts[3]))
    return fams, faces


def names_check():
    print("families: fontdb (fonttest --scan) vs GDI EnumFontFamiliesExW")
    out = OUT / "scan.txt"
    t0 = time.time()
    fonttest("--scan", out)
    ours, faces = parse_scan(out.read_text(encoding="utf-8"))
    gdi = gdi_families()
    # variable fonts: GDI lists their named instances as extra families
    var_fams = {f["tfamily"].lower() for f in faces if f["flags"] & 32}
    var_fams |= {f["family"].lower() for f in faces if f["flags"] & 32}
    only_gdi = sorted(v for k, v in gdi.items() if k not in ours)
    only_ours = sorted(v for k, v in ours.items() if k not in gdi and k not in var_fams)
    explained = [n for n in only_gdi if any(n.lower().startswith(v) for v in var_fams)]
    unexplained = [n for n in only_gdi if n not in explained]
    print(f"  fontdb {len(ours)} families ({len(faces)} faces), GDI {len(gdi)} TrueType/OpenType families")
    print(f"  common {len(set(ours) & set(gdi))}; GDI only: {len(only_gdi)} ({len(explained)} named instances"
          f" of variable fonts); fontdb only: {len(only_ours)}")
    for n in unexplained[:20]:
        print(f"    GDI only: {n}")
    for n in only_ours[:20]:
        print(f"    fontdb only: {n}")
    ok = len(unexplained) <= max(2, len(gdi) // 50) and len(only_ours) <= max(2, len(ours) // 50)
    print(f"  {'OK  ' if ok else 'FAIL'} family lists agree")
    return ok


def read_png(path):
    """minimal PNG reader (8-bit RGB / RGBA, non-interlaced) -> (w, h, rows of (r, g, b, a))"""
    import zlib
    d = Path(path).read_bytes()
    pos, idat, w = 8, b"", 0
    while pos < len(d):
        n, t = struct.unpack_from(">I4s", d, pos)
        c = d[pos + 8:pos + 8 + n]
        pos += 12 + n
        if t == b"IHDR":
            w, h, depth, ctype = struct.unpack_from(">IIBB", c)
            bpp = {6: 4, 2: 3}[ctype]
        elif t == b"IDAT":
            idat += c
    raw = zlib.decompress(idat)
    stride = w * bpp
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        rows.append([tuple(line[i:i + bpp]) + ((255,) if bpp == 3 else ()) for i in range(0, stride, bpp)])
        prev = line
    return w, h, rows


def make_png(w, h, pixel):
    import zlib
    raw = b"".join(b"\0" + b"".join(bytes(pixel(x, y)) for x in range(w)) for y in range(h))

    def chunk(t, c):
        return struct.pack(">I", len(c)) + t + c + struct.pack(">I", zlib.crc32(t + c) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) +
            chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def make_cbdt_font(path):
    """A small colour bitmap font: glyph 1 ('A') = red square, glyph 2 (U+1F600) = blue disc,
    one 32 ppem strike (CBLC index format 1, CBDT image format 17), no outlines."""
    upem, n = 1024, 3
    red = make_png(32, 32, lambda x, y: (220, 30, 30, 255) if 4 <= x < 28 and 4 <= y < 28 else (0, 0, 0, 0))

    def disc(x, y):
        d = ((x - 15.5) ** 2 + (y - 15.5) ** 2) ** 0.5
        return (20, 60, 230, 255) if d < 14 else (0, 0, 0, 0)
    blue = make_png(32, 32, disc)
    imgs = []
    for png in (red, blue):
        imgs.append(struct.pack(">BBbbB", 32, 32, 0, 28, 32) + struct.pack(">I", len(png)) + png)
    cbdt = struct.pack(">HH", 3, 0) + b"".join(imgs)
    offs = [0, len(imgs[0]), len(imgs[0]) + len(imgs[1])]
    sub = struct.pack(">HHI", 1, 17, 4) + b"".join(struct.pack(">I", o) for o in offs)
    arr = struct.pack(">HHI", 1, 2, 8)
    line = struct.pack(">bbBbbbbbbbbb", 28, -4, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0)
    size = (struct.pack(">IIII", 8 + 48, len(arr) + len(sub), 1, 0) + line + line +
            struct.pack(">HHBBBb", 1, 2, 32, 32, 32, 1))
    cblc = struct.pack(">HHI", 3, 0, 1) + size + arr + sub
    head = struct.pack(">IIIIHHqqhhhhHHhhh", 0x10000, 0x10000, 0, 0x5F0F3CF5, 0, upem, 0, 0,
                       0, -256, 1024, 896, 0, 8, 2, 0, 0)
    hhea = struct.pack(">IhhhHhhhhhhhhhhhH", 0x10000, 896, -128, 0, 1024, 0, 0, 1024, 1, 0, 0,
                       0, 0, 0, 0, 0, n)
    maxp = struct.pack(">IH", 0x5000, n)
    hmtx = b"".join(struct.pack(">Hh", 1024, 0) for _ in range(n))
    # cmap: format 12 (3,10) with 'A' -> 1, U+1F600 -> 2
    groups = [(0x41, 0x41, 1), (0x1F600, 0x1F600, 2)]
    f12 = struct.pack(">HHIII", 12, 0, 16 + 12 * len(groups), 0, len(groups)) + \
        b"".join(struct.pack(">III", *g) for g in groups)
    cmap = struct.pack(">HHHHI", 0, 1, 3, 10, 12) + f12
    fam = "CBDT Test".encode("utf-16-be")
    sty = "Regular".encode("utf-16-be")
    recs = [(1, fam), (2, sty), (4, fam)]
    strs, rec = b"", b""
    for nid, v in recs:
        rec += struct.pack(">HHHHHH", 3, 1, 0x409, nid, len(v), len(strs))
        strs += v
    name = struct.pack(">HHH", 0, len(recs), 6 + 12 * len(recs)) + rec + strs
    os2 = struct.pack(">HhHHH", 4, 512, 400, 5, 0) + bytes(96 - 10)
    post = struct.pack(">IihhI", 0x30000, 0, -100, 50, 0) + bytes(16)
    tables = {"CBDT": cbdt, "CBLC": cblc, "OS/2": os2, "cmap": cmap, "head": head, "hhea": hhea,
              "hmtx": hmtx, "maxp": maxp, "name": name, "post": post}
    tags = sorted(tables)
    off = 12 + 16 * len(tags)
    dirs, body = b"", b""
    for t in tags:
        data = tables[t]
        dirs += t.encode("latin-1") + struct.pack(">III", 0, off + len(body), len(data))
        body += data + bytes((-len(data)) % 4)
    out = struct.pack(">IHHHH", 0x10000, len(tags), 0, 0, 0) + dirs + body
    Path(path).write_bytes(out)


def cbdt_check():
    print("CBDT colour bitmaps (synthetic test font)")
    f = OUT / "cbdt_test.ttf"
    make_cbdt_font(f)
    ok = True
    for px in (16, 32, 64):
        out = OUT / f"render_cbdt_{px}.png"
        rc, msg = fonttest("--render", f, 0, px, "A\U0001F600A", out, "--bg", "FFFFFF", check=False)
        if rc != 0:
            print(f"  FAIL {px} px: fonttest {rc}: {msg.strip()[-200:]}")
            ok = False
            continue
        w, h, rows = read_png(out)
        red = sum(1 for r in rows for p in r if p[0] > 180 and p[1] < 90 and p[2] < 90)
        blue = sum(1 for r in rows for p in r if p[2] > 180 and p[0] < 90)
        side = 24 * px / 32
        want_red, want_blue = 2 * side * side, 3.14159 * (14 * px / 32) ** 2
        good = abs(red - want_red) < 0.25 * want_red + 8 and abs(blue - want_blue) < 0.3 * want_blue + 8
        print(f"  {'OK  ' if good else 'FAIL'} {px} px: {red} red px (~{want_red:.0f}), {blue} blue px (~{want_blue:.0f})")
        ok &= good
    return ok


def colr_check():
    """COLR layers: Segoe UI Emoji renders coloured pixels, a plain font does not"""
    p = WINFONTS / "seguiemj.ttf"
    if not p.exists():
        return True
    out = OUT / "render_colr_check.png"
    rc, _ = fonttest("--render", p, 0, 48, "\U0001F600\U0001F98A", out, check=False)
    if rc != 0:
        print("  FAIL COLR render")
        return False
    w, h, rows = read_png(out)
    col = sum(1 for r in rows for q in r if max(q[:3]) - min(q[:3]) > 60)
    ok = col > 400
    print(f"  {'OK  ' if ok else 'FAIL'} COLR emoji: {col} coloured pixels")
    return ok


def linux_check():
    """fontdb in WSL vs fc-list; the same TTF renders identically on both targets"""
    print("linux (WSL): fontdb vs fc-list")
    fc = subprocess.run(["wsl.exe", "-d", WSL_DISTRO, "-e", "fc-list", ":", "family", "file"],
                        capture_output=True)
    if fc.returncode != 0:
        print("  SKIP fc-list not available")
        return True
    fcmap = {}
    for line in fc.stdout.decode("utf-8", "replace").splitlines():
        if ": " not in line:
            continue
        path, fams = line.split(": ", 1)
        fcmap.setdefault(path.strip(), set()).update(x.strip().lower() for x in fams.split(","))
    out = OUT / "scan_linux.txt"
    rc, msg = fonttest("--scan", out, target="linux", check=False)
    if rc != 0:
        print(f"  FAIL fonttest --scan on Linux: {rc} {msg.strip()[-200:]}")
        return False
    ours, faces = parse_scan(out.read_text(encoding="utf-8"))
    # the same font may be reached through a symbolic link: compare resolved paths
    paths = sorted({f["path"] for f in faces} | set(fcmap))
    rl = subprocess.run(["wsl.exe", "-d", WSL_DISTRO, "-e", "readlink", "-f"] + paths, capture_output=True)
    real = dict(zip(paths, rl.stdout.decode("utf-8", "replace").splitlines()))
    fcmap = {real.get(k, k): v for k, v in fcmap.items()}
    for f in faces:
        f["path"] = real.get(f["path"], f["path"])
    by_path = {}
    for f in faces:
        by_path.setdefault(f["path"], []).append(f)
    bad = []
    for f in faces:
        fams = fcmap.get(f["path"])
        if fams is None:
            continue
        if f["family"].lower() not in fams and f["tfamily"].lower() not in fams:
            bad.append(f"{f['path']}: ours '{f['family']}' / '{f['tfamily']}', fc {sorted(fams)}")
    missing = [p for p in fcmap if p not in by_path]
    extra = [p for p in by_path if p not in fcmap]
    print(f"  fc-list {len(fcmap)} files, fontdb {len(by_path)} files ({len(faces)} faces, {len(ours)} families)")
    for b in bad[:10]:
        print("    name mismatch:", b)
    for m in missing[:10]:
        print("    not in fontdb:", m)
    for e in extra[:10]:
        print("    not in fc-list:", e)
    ok = not bad and not missing
    print(f"  {'OK  ' if ok else 'FAIL'} names and files agree")
    # identical rendering on both targets (same code, same file)
    same = True
    for f, px, text in (("arial.ttf", 20, "Linux == Windows AVATAR"), ("seguiemj.ttf", 32, "\U0001F600\U0001F98A")):
        src = WINFONTS / f
        if not src.exists():
            continue
        a, b = OUT / f"xplat_win_{Path(f).stem}.png", OUT / f"xplat_lx_{Path(f).stem}.png"
        fonttest("--render", src, 0, px, text, a, check=False)
        fonttest("--render", src, 0, px, text, b, target="linux", check=False)
        eq = a.exists() and b.exists() and read_png(a) == read_png(b)
        print(f"  {'OK  ' if eq else 'FAIL'} {f}: Windows and Linux renders identical")
        same &= eq
    return ok and same


if __name__ == "__main__":
    sys.exit(main())
