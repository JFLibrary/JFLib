"""pngutil.py -- tiny PNG helpers for the packaging tools (no third-party modules).

  read_png(path)              -> (w, h, bytearray RGBA)
  png_decode(data)            -> the same from the bytes of a PNG file
  write_png(path, w, h, rgba) -- 8-bit RGBA, filter 0 per row (compressed with zlib)
  png_bytes(w, h, rgba)       -> bytes of a PNG file
  downscale(w, h, rgba, nw, nh) -> bytearray RGBA (area average; nw <= w, nh <= h)
  round_corners(w, h, rgba, radius) -- multiply alpha by a 4x4 supersampled rounded square
"""
import struct
import zlib


def read_png(path):
    with open(path, "rb") as f:
        return png_decode(f.read(), path)


def png_decode(data, path="PNG"):
    pos, w, h, idat, bpp = 8, 0, 0, b"", 4
    while pos < len(data):
        n, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if typ == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
            if depth != 8 or ctype not in (2, 6):
                raise ValueError(f"{path}: only 8-bit RGB / RGBA PNGs")
            bpp = {2: 3, 6: 4}[ctype]
        elif typ == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = bytearray(w * h * 4)
    prev = bytearray(stride)
    i = 0
    for y in range(h):
        f = raw[i]
        line = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        if f:
            for x in range(stride):
                a = line[x - bpp] if x >= bpp else 0
                b = prev[x]
                if f == 1:
                    line[x] = (line[x] + a) & 255
                elif f == 2:
                    line[x] = (line[x] + b) & 255
                elif f == 3:
                    line[x] = (line[x] + (a + b) // 2) & 255
                elif f == 4:
                    c = prev[x - bpp] if x >= bpp else 0
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        o = y * w * 4
        if bpp == 4:
            out[o:o + w * 4] = line
        else:
            for x in range(w):
                out[o + x * 4:o + x * 4 + 3] = line[x * 3:x * 3 + 3]
                out[o + x * 4 + 3] = 255
        prev = line
    return w, h, out


def png_bytes(w, h, rgba):
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        raw += rgba[y * w * 4:(y + 1) * w * 4]

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def write_png(path, w, h, rgba):
    with open(path, "wb") as f:
        f.write(png_bytes(w, h, rgba))


def downscale(w, h, rgba, nw, nh):
    out = bytearray(nw * nh * 4)
    for ny in range(nh):
        y0, y1 = ny * h // nh, max((ny + 1) * h // nh, ny * h // nh + 1)
        for nx in range(nw):
            x0, x1 = nx * w // nw, max((nx + 1) * w // nw, nx * w // nw + 1)
            acc = [0, 0, 0, 0]
            for y in range(y0, y1):
                row = y * w * 4
                for x in range(x0, x1):
                    p = row + x * 4
                    a = rgba[p + 3]
                    acc[0] += rgba[p] * a
                    acc[1] += rgba[p + 1] * a
                    acc[2] += rgba[p + 2] * a
                    acc[3] += a
            n = (y1 - y0) * (x1 - x0)
            o = (ny * nw + nx) * 4
            if acc[3]:
                out[o] = acc[0] // acc[3]
                out[o + 1] = acc[1] // acc[3]
                out[o + 2] = acc[2] // acc[3]
            out[o + 3] = (acc[3] + n // 2) // n
    return out


def round_corners(w, h, rgba, radius):
    r = float(radius)
    for y in range(h):
        for x in range(w):
            # only the corner squares need work
            if (x >= r and x < w - r) or (y >= r and y < h - r):
                continue
            cx = r if x < r else w - r
            cy = r if y < r else h - r
            inside = 0
            for sy in range(4):
                for sx in range(4):
                    px = x + (sx + 0.5) / 4
                    py = y + (sy + 0.5) / 4
                    if (px - cx) ** 2 + (py - cy) ** 2 <= r * r:
                        inside += 1
            p = (y * w + x) * 4 + 3
            rgba[p] = rgba[p] * inside // 16
