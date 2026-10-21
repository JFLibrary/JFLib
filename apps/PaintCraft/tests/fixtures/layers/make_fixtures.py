"""Generates the small test images of the layers package (agent A4; run from anywhere):
l4_small.png  32 x 24 RGBA: left half opaque red, right half green at alpha 128,
              pixel (31, 23) transparent
l4_wide.png   96 x 40 RGB-A: opaque blue, pixel (95, 0) yellow, pixel (0, 39) white
l4_tall.png   40 x 80 RGBA: opaque yellow, pixel (0, 79) black
l4_lo.png     4 x 2 RGBA: the lower colours of the blend mode test (layer_props.uis), by x:
              (200,100,50,255) (30,180,220,255) (128,64,192,255) (90,90,90,200)
l4_hi.png     4 x 2 RGBA: the upper colours, by x:
              (60,150,240,200) (220,40,100,255) (100,200,30,128) (250,10,130,255)
The .pdn fixtures are made by the tests themselves (Speichern unter .pdn of a document
built by the script: build/a4_layers.pdn) or come from paint.net
(apps/PaintCraft/tests/fixtures/pdn/c_layers.pdn).
"""
import pathlib, struct, zlib

HERE = pathlib.Path(__file__).resolve().parent


def chunk(kind, data):
    c = struct.pack(">I", len(data)) + kind + data
    return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def png(path, w, h, pixel):
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for x in range(w):
            raw += bytes(pixel(x, y))
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
    data += chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    path.write_bytes(data)


def small(x, y):
    if (x, y) == (31, 23):
        return (0, 0, 0, 0)
    return (255, 0, 0, 255) if x < 16 else (0, 255, 0, 128)


def wide(x, y):
    if (x, y) == (95, 0):
        return (255, 255, 0, 255)
    if (x, y) == (0, 39):
        return (255, 255, 255, 255)
    return (0, 0, 255, 255)


def tall(x, y):
    return (0, 0, 0, 255) if (x, y) == (0, 79) else (255, 255, 0, 255)


LO = [(200, 100, 50, 255), (30, 180, 220, 255), (128, 64, 192, 255), (90, 90, 90, 200)]
HI = [(60, 150, 240, 200), (220, 40, 100, 255), (100, 200, 30, 128), (250, 10, 130, 255)]


if __name__ == "__main__":
    png(HERE / "l4_small.png", 32, 24, small)
    png(HERE / "l4_wide.png", 96, 40, wide)
    png(HERE / "l4_tall.png", 40, 80, tall)
    png(HERE / "l4_lo.png", 4, 2, lambda x, y: LO[x])
    png(HERE / "l4_hi.png", 4, 2, lambda x, y: HI[x])
