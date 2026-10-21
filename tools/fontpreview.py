#!/usr/bin/env python3
"""fontpreview.py -- preview sheet of the SGF1 font/icon assets (rendered by refrender.py).

  python tools/fontpreview.py                       -> build/fontpreview.png
         [--out FILE] [--section all|text|charset|kern|icons] [--mode ss16|area]
  python tools/fontpreview.py --scan E700-E7FF --out sheet.png [--font SegoeIcons.ttf]
         candidate sheet: every glyph of a TrueType font in a codepoint range, labelled with
         its hex codepoint (used to pick icons for tools/icons.txt)
"""
import argparse
import os
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, TOOLS)
import refrender as rr  # noqa: E402

ROOT = os.path.dirname(TOOLS)
ASSETS = os.path.join(ROOT, "legacy", "ASMLIB", "styles", "winui", "assets")
FONTS = (("font_reg.bin", "Regular (segoeui)"), ("font_semi.bin", "Semibold (seguisb)"),
         ("font_bold.bin", "Bold (segoeuib)"))
SIZES = (12, 14, 20, 28)
SAMPLES = (
    "The quick brown fox jumps over the lazy dog. 0123456789 (+-*/=) [a|b] {c} #@&%$",
    "Größe Ärger Öl Übel Maß äöüß ÄÖÜ „Anführung“ “quotes” ‘single’ «guillemets» ‹›",
    "€ 12,50 – — … ™ № ‰ † ‡ • ← ↑ → ↓ − × ÷ ≈ ≠ ≤ ≥ ∞ ● � ƒ Ș ș Ț ț ˆ ˜ © ® ° ± µ ¶ §",
    "Łódź Český Kraków Ångström Œuvre Ÿ ĳ ŉ ő ű Ģ ķ ŗ ş ţ ž Ž č ć đ ł ń ś ź ż ą ę ı",
)
KERN = "AVATAR WAVE Tower Yoga LT P. F, Vo Ty Wa \"A\" 'Tj' r. y,"
GRAY = (110, 110, 110)
BLUE = (0, 90, 180)


def load(name):
    return rr.SGFFont(os.path.join(ASSETS, name))


def section_text(mode):
    lab = load("font_reg.bin")
    W = 1400
    blocks = []
    for fname, title in FONTS:
        f = load(fname)
        h = 30 + sum(int(px * 1.45) * len(SAMPLES) + 10 for px in SIZES)
        cv = rr.Canvas(W, h)
        cv.text(lab, f"{fname} -- {title}  (size64 = px*64, sy snapped to x-height for px <= 24)",
                14, 10, 20, BLUE)
        y = 30
        for px in SIZES:
            lh = int(px * 1.45)
            for s in SAMPLES:
                y += lh
                cv.text(lab, f"{px}px", 11, 8, y - int(px * 0.3), GRAY)
                cv.text(f, s, px, 50, y - int(px * 0.3), mode=mode)
            y += 10
        blocks.append(cv)
    return blocks


def section_kern(mode):
    lab = load("font_reg.bin")
    W = 1400
    cv = rr.Canvas(W, 3 * 2 * 44 + 40)
    cv.text(lab, "Kerning: upper line kerned (class matrix), lower line not kerned; 28 px", 14, 10, 20, BLUE)
    y = 40
    for fname, title in FONTS:
        f = load(fname)
        for kern in (True, False):
            y += 40
            cv.text(lab, ("kern " if kern else "none ") + fname[5:-4], 11, 8, y - 8, GRAY)
            wpx = cv.text(f, KERN, 28, 90, y - 8, kerning=kern, mode=mode)
            cv.text(lab, f"{wpx:.2f} px", 11, 100 + int(wpx) + 10, y - 8, GRAY)
        y += 4
    return [cv]


def section_charset(mode, px=22):
    lab = load("font_reg.bin")
    out = []
    for fname, title in FONTS:
        f = load(fname)
        items = list(f.codepoints())
        cols, cw, ch = 30, 46, 50
        rows = (len(items) + cols - 1) // cols
        cv = rr.Canvas(cols * cw + 1, rows * ch + 30)
        cv.text(lab, f"{fname}: all {len(items)} codepoints at {px} px (label = hex codepoint; "
                f"red line = advance)", 14, 10, 20, BLUE)
        k = rr.fx_scale(f, rr.size64_of(px))
        for i, (cp, gid) in enumerate(items):
            x, y = (i % cols) * cw, 30 + (i // cols) * ch
            cv.frame(x, y, cw + 1, ch + 1, (220, 220, 220))
            cv.text(lab, f"{cp:04X}", 9, x + 2, y + 10, GRAY)
            adv = rr.fu_to_fx(f.advance(gid), k) / 256
            gx = x + 8
            base = y + 40
            cv.rect(gx, base, max(1, int(round(adv))), 1, (240, 150, 150))
            cv.text(f, [cp], px, gx, base, mode=mode)
        out.append(cv)
    return out


def section_icons(mode):
    lab = load("font_reg.bin")
    f = load("icons.bin")
    names = {}
    import ttf2bin
    for name, cp in ttf2bin.read_icons(os.path.join(TOOLS, "icons.txt")):
        names[cp] = name
    items = sorted(names.items(), key=lambda t: list(names).index(t[0]))
    cols, cw, ch = 10, 140, 74
    rows = (len(items) + cols - 1) // cols
    cv = rr.Canvas(cols * cw + 1, rows * ch + 30)
    cv.text(lab, f"icons.bin: {len(items)} icons, 32 px and 16 px (name, IC_ constant hex)", 14, 10, 20, BLUE)
    for i, (cp, name) in enumerate(items):
        x, y = (i % cols) * cw, 30 + (i // cols) * ch
        cv.frame(x, y, cw + 1, ch + 1, (220, 220, 220))
        cv.text(f, [cp], 32, x + 8, y + 38, mode=mode)
        cv.text(f, [cp], 16, x + 50, y + 30, mode=mode)
        cv.text(lab, f"{cp:04X}", 11, x + 76, y + 24, GRAY)
        cv.text(lab, name[:22], 11, x + 6, y + 64, (0, 0, 0))
    return [cv]


def stack(blocks, gap=8):
    W = max(b.w for b in blocks)
    H = sum(b.h for b in blocks) + gap * (len(blocks) - 1)
    cv = rr.Canvas(W, H, (235, 235, 235))
    y = 0
    for b in blocks:
        cv.px[y:y + b.h, :b.w] = b.px
        y += b.h + gap
    return cv


def scan(font_path, spec, out, px=32, mode="ss16"):
    import ttf2bin
    tf = ttf2bin.TTFont(font_path)
    cmap = tf.cmap()[0]
    cps = [c for c in ttf2bin.parse_charset(spec) if c in cmap]
    img, _ = ttf2bin.build_sgf(tf, cps, icon=True, fallback_cp=cps[0])
    f = rr.SGFFont(img)
    lab = load("font_reg.bin")
    cw, ch = max(86, int(px * 1.8)), px + 40
    cols = max(1, min(16, 1400 // cw))
    rows = (len(cps) + cols - 1) // cols
    cv = rr.Canvas(cols * cw + 1, rows * ch + 1)
    for i, cp in enumerate(cps):
        x, y = (i % cols) * cw, (i // cols) * ch
        cv.frame(x, y, cw + 1, ch + 1, (200, 200, 200))
        cv.text(lab, f"{cp:04X}", 13, x + 4, y + 14, BLUE)
        cv.text(f, [cp], px, x + (cw - px) // 2, y + 18 + px, mode=mode)
    cv.save(out)
    print(f"{out}: {len(cps)} glyphs of {os.path.basename(font_path)} in {spec}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default=os.path.join(ROOT, "build", "fontpreview.png"))
    ap.add_argument("--section", default="all", choices=("all", "text", "charset", "kern", "icons"))
    ap.add_argument("--mode", default="ss16", choices=("ss16", "area"))
    ap.add_argument("--scan", help="hex range(s) of a TTF to put on a candidate sheet")
    ap.add_argument("--font", default=os.path.join(os.environ.get("WINDIR", r"C:\Windows"), "Fonts",
                                                   "SegoeIcons.ttf"))
    ap.add_argument("--px", type=int, default=32)
    a = ap.parse_args()
    if a.scan:
        scan(a.font, a.scan, a.out, a.px, a.mode)
        return 0
    blocks = []
    if a.section in ("all", "text"):
        blocks += section_text(a.mode)
    if a.section in ("all", "kern"):
        blocks += section_kern(a.mode)
    if a.section in ("all", "icons"):
        blocks += section_icons(a.mode)
    if a.section in ("all", "charset"):
        blocks += section_charset(a.mode)
    cv = stack(blocks)
    cv.save(a.out)
    print(f"{a.out}: {cv.w}x{cv.h}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
