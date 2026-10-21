"""mkicon.py -- app icons: renders a glyph of the icon font in white on a gradient square in every
icon size, cuts anti-aliased rounded corners and writes

  apps/NAME/res/app.ico          16 .. 256 px (PNG entries), linked into the Windows exe (build.py)
  apps/NAME/res/app_SIZE.png     48, 128, 256 px for Linux (hicolor icon theme, .deb)

  python tools/mkicon.py NAME --glyph E771 --color 1E78E6

The glyph is an IC_* code point (lib/ui/keys.rs) of styles/winui/assets/icons.bin (Fluent UI
System Icons, tools/icons_fluent.txt); the committed icons are installer E896 2E9E5B, PaintCraft
E771 1E78E6 and Kritzel EB7E D9822B.  tools/refrender.py (the reference of the library's glyph
rasterizer) draws them with the layout of the former assembly icongen app: 48 bands from the
colour mixed with 28 % white (top) to the colour (bottom), the glyph centred (GLYPH below).
"""
import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pngutil  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
LINUX_SIZES = [48, 128, 256]
ICONS = ROOT / "styles" / "winui" / "assets" / "icons.bin"
BANDS = 48                      # gradient bands (top lighter, bottom the colour)
GLYPH = 72                      # glyph em, % of the side: icongen drew Segoe Fluent Icons at 62 %;
                                # the Fluent 16 px designs keep a 1 px margin (median 86 % of the
                                # Segoe glyph size), so 72 % gives the former visual size


def ico_bytes(images):
    """images: list of (size, png bytes) -> .ico file"""
    head = struct.pack("<HHH", 0, 1, len(images))
    off = 6 + 16 * len(images)
    dirs, data = b"", b""
    for size, png in images:
        b = 0 if size >= 256 else size
        dirs += struct.pack("<BBBBHHII", b, b, 0, 0, 1, 32, len(png), off + len(data))
        data += png
    return head + dirs + data


def mix(a, b, t):
    """colour a -> b by t / 256 per channel (ui color_mix)"""
    return tuple((x * (256 - t) + y * t + 128) >> 8 for x, y in zip(a, b))


def render(font, cp, s, color):
    """s x s RGBA bytearray: the gradient square with the white glyph cp centred on it"""
    import numpy as np
    import refrender
    top = mix(color, (255, 255, 255), 72)
    cv = refrender.Canvas(s, s)
    for i in range(BANDS):
        y0, y1 = s * i // BANDS, s * (i + 1) // BANDS
        cv.rect(0, y0, s, y1 - y0, mix(top, color, i * 256 // (BANDS - 1)))
    g = s * GLYPH // 100
    o = (s - g) >> 1
    cv.text(font, [cp], g, o, o + g, (255, 255, 255))      # icon em box: top o, baseline o + g
    rgb = np.clip(np.rint(cv.px), 0, 255).astype(np.uint8)
    return bytearray(np.dstack([rgb, np.full((s, s), 255, np.uint8)]).tobytes())


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("app")
    ap.add_argument("--glyph", default="E771", help="icon code point (hex, IC_* in lib/ui/keys.rs)")
    ap.add_argument("--color", default="1E78E6", help="square colour RRGGBB")
    ap.add_argument("--font", default=str(ICONS), help="SGF1 icon font (default: winui's icons.bin)")
    a = ap.parse_args()
    import refrender
    font = refrender.SGFFont(a.font)
    cp = int(a.glyph, 16)
    if font.lookup(cp) is None:
        raise SystemExit(f"U+{cp:04X} is not in {a.font}")
    c = int(a.color, 16)
    color = (c >> 16 & 255, c >> 8 & 255, c & 255)
    res = ROOT / "apps" / a.app / "res"
    res.mkdir(parents=True, exist_ok=True)
    images = []
    for s in SIZES:
        px = render(font, cp, s, color)
        pngutil.round_corners(s, s, px, max(2.0, s * 0.22))
        png = pngutil.png_bytes(s, s, px)
        images.append((s, png))
        if s in LINUX_SIZES:
            (res / f"app_{s}.png").write_bytes(png)
    (res / "app.ico").write_bytes(ico_bytes(images))
    print(f"mkicon: {res / 'app.ico'} ({len(images)} sizes), app_{{{','.join(map(str, LINUX_SIZES))}}}.png")


if __name__ == "__main__":
    main()
