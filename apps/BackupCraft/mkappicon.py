"""mkappicon.py -- BackupCraft's program icon (own artwork) -> res/app.ico, res/app_{48,128,256}.png

    python apps/BackupCraft/mkappicon.py

A petrol square (the style's accent, a soft vertical gradient) with a white shield and a petrol
check mark -- the shield of the console's sign-in panel (BC_SHIELD_CHECK, mkicons.py).  Drawn
here on a 1024 px master with scanline coverage, every size box-filtered from it, the corners
rounded like tools/mkicon.py does."""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent / "tools"))
import pngutil  # noqa: E402
from mkicon import LINUX_SIZES, SIZES, ico_bytes  # noqa: E402

M = 1024                      # master size
SS = 2                        # subsamples per pixel row (scanline coverage is exact across x)
TOP, BOTTOM = (0x12, 0x8C, 0xA1), (0x08, 0x5A, 0x6A)
CHECK = (0x0B, 0x72, 0x85)


def cubic(p0, p1, p2, p3, n=24):
    out = []
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        out.append((u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                    u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]))
    return out


def shield():
    """the shield of mkicons.py on the 24 grid, as a polygon"""
    pts = [(12, 3.2), (19.2, 6), (19.2, 11.6)]
    pts += cubic((19.2, 11.6), (19.2, 16), (16.2, 19.4), (12, 20.8))
    pts += cubic((12, 20.8), (7.8, 19.4), (4.8, 16), (4.8, 11.6))
    pts += [(4.8, 6)]
    return pts


def to_master(pts, scale, ox, oy):
    return [(ox + x * scale, oy + y * scale) for x, y in pts]


def coverage_polygon(poly):
    """{row: [(x0, x1) spans]} per subsample row of the master"""
    rows = {}
    n = len(poly)
    for sy in range(M * SS):
        y = (sy + 0.5) / SS
        xs = []
        for i in range(n):
            (x0, y0), (x1, y1) = poly[i], poly[(i + 1) % n]
            if (y0 <= y < y1) or (y1 <= y < y0):
                xs.append(x0 + (y - y0) * (x1 - x0) / (y1 - y0))
        xs.sort()
        if xs:
            rows[sy] = list(zip(xs[0::2], xs[1::2]))
    return rows


def blend(px, x, y, rgb, a):
    if a <= 0:
        return
    p = (y * M + x) * 4
    for c in range(3):
        px[p + c] = round(px[p + c] * (1 - a) + rgb[c] * a)


def paint_spans(px, rows, rgb):
    acc = {}
    for sy, spans in rows.items():
        y = sy // SS
        for x0, x1 in spans:
            xa, xb = max(0.0, x0), min(float(M), x1)
            ix = int(xa)
            while ix < xb:
                c = min(xb, ix + 1) - max(xa, ix)
                if c > 0:
                    acc[(ix, y)] = acc.get((ix, y), 0.0) + c / SS
                ix += 1
    for (x, y), a in acc.items():
        blend(px, x, y, rgb, min(1.0, a))


def paint_stroke(px, pts, width, rgb):
    """a polyline with round caps and joins (distance coverage, 4x4 samples)"""
    r = width / 2
    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
    for y in range(int(min(ys) - r) - 1, int(max(ys) + r) + 2):
        for x in range(int(min(xs) - r) - 1, int(max(xs) + r) + 2):
            inside = 0
            for sy in range(4):
                for sx in range(4):
                    qx, qy = x + (sx + 0.5) / 4, y + (sy + 0.5) / 4
                    best = 1e9
                    for (ax, ay), (bx, by) in zip(pts, pts[1:]):
                        dx, dy = bx - ax, by - ay
                        t = max(0.0, min(1.0, ((qx - ax) * dx + (qy - ay) * dy) / (dx * dx + dy * dy)))
                        ex, ey = ax + t * dx - qx, ay + t * dy - qy
                        best = min(best, ex * ex + ey * ey)
                    inside += best <= r * r
            blend(px, x, y, rgb, inside / 16)


def master():
    px = bytearray(M * M * 4)
    for y in range(M):
        t = y / (M - 1)
        rgb = [round(TOP[c] + (BOTTOM[c] - TOP[c]) * t) for c in range(3)]
        for x in range(M):
            p = (y * M + x) * 4
            px[p:p + 4] = bytes(rgb + [255])
    # the 24 grid fills 74 % of the square, centred
    scale = M * 0.74 / 24
    ox = oy = (M - 24 * scale) / 2
    paint_spans(px, coverage_polygon(to_master(shield(), scale, ox, oy)), (255, 255, 255))
    paint_stroke(px, to_master([(8.4, 12.1), (11, 14.6), (15.8, 9.6)], scale, ox, oy), 2.1 * scale, CHECK)
    return px


def main():
    res = HERE / "res"
    res.mkdir(exist_ok=True)
    big = master()
    images = []
    for s in SIZES:
        px = pngutil.downscale(M, M, big, s, s)
        pngutil.round_corners(s, s, px, max(2.0, s * 0.22))
        png = pngutil.png_bytes(s, s, px)
        images.append((s, png))
        if s in LINUX_SIZES:
            (res / f"app_{s}.png").write_bytes(png)
    (res / "app.ico").write_bytes(ico_bytes(images))
    print(f"mkappicon: {res / 'app.ico'} ({len(images)} sizes), app_{{{','.join(map(str, LINUX_SIZES))}}}.png")


if __name__ == "__main__":
    main()
