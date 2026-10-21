#!/usr/bin/env python3
# framecmp.py FRAME PNG -- compare a captured frame (wlinject.py: u32 w, h, BGRX) with a PNG
# (8-bit RGB / RGBA) -> differing pixels
import struct, sys, zlib


def read_png(path):
    d = open(path, "rb").read()
    assert d[:8] == b"\x89PNG\r\n\x1a\n"
    o, idat, w = 8, b"", 0
    while o < len(d):
        n, t = struct.unpack(">I4s", d[o:o + 8])
        c = d[o + 8:o + 8 + n]
        if t == b"IHDR":
            w, h, bd, ct = struct.unpack(">IIBB", c[:10])
        elif t == b"IDAT":
            idat += c
        o += 12 + n
    bpp = {2: 3, 6: 4}[ct]
    raw = zlib.decompress(idat)
    stride = w * bpp
    rows, prev, p = [], bytearray(stride), 0
    for y in range(h):
        f = raw[p]
        cur = bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for x in range(stride):
            a = cur[x - bpp] if x >= bpp else 0
            b = prev[x]
            cc = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                cur[x] = (cur[x] + a) & 255
            elif f == 2:
                cur[x] = (cur[x] + b) & 255
            elif f == 3:
                cur[x] = (cur[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - cc), abs(a - cc), abs(a + b - 2 * cc)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else cc)
                cur[x] = (cur[x] + pr) & 255
        rows.append(bytes(cur))
        prev = cur
    return w, h, bpp, rows


fr = open(sys.argv[1], "rb").read()
fw, fh = struct.unpack("<II", fr[:8])
px = fr[8:]
w, h, bpp, rows = read_png(sys.argv[2])
if (fw, fh) != (w, h):
    print(f"size differs: frame {fw}x{fh}, png {w}x{h}")
    sys.exit(1)
bad = 0
for y in range(h):
    for x in range(w):
        b, g, r = px[(y * w + x) * 4:(y * w + x) * 4 + 3]
        if rows[y][x * bpp:x * bpp + 3] != bytes((r, g, b)):
            if bad < 5:
                print("pixel", x, y, "frame", (r, g, b), "png", tuple(rows[y][x * bpp:x * bpp + 3]))
            bad += 1
print(f"{w}x{h}: {bad} pixels differ")
sys.exit(1 if bad else 0)
