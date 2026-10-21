"""Generates the small test images of apps/PaintCraft/tests (run from the repository root):
a1_rgba.png  64 x 48 RGBA, 300 dpi: red / half transparent green / blue / white quadrants,
             an 8 x 8 transparent hole at (48, 32)
a1_rgb.jpg, a1_rgb.bmp, a1_pal.gif  opaque variants (ffmpeg), for the format round trips
a1_rgba.heic the RGBA image as lossless 4:4:4 HEVC (identity matrix, full range) with an alpha
             item, rotated by 180 degrees through irot (writer of tools/heic_check.py, x265)
a1_rgba.avif likewise as lossless AV1 (aom)
a1_rgba.jxl  the RGBA image as lossless JPEG XL (ffmpeg libjxl)
"""
import pathlib, struct, subprocess, zlib

HERE = pathlib.Path(__file__).resolve().parent
W, H = 64, 48


def pixel(x, y):
    if 48 <= x < 56 and 32 <= y < 40:
        return (0, 0, 0, 0)
    if y < 24:
        return (255, 0, 0, 255) if x < 32 else (0, 255, 0, 128)
    return (0, 0, 255, 255) if x < 32 else (255, 255, 255, 255)


def chunk(kind, data):
    c = struct.pack(">I", len(data)) + kind + data
    return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def png(path, alpha=True):
    raw = bytearray()
    for y in range(H):
        raw.append(0)
        for x in range(W):
            p = pixel(x, y)
            raw += bytes(p if alpha else (p[0], p[1], p[2]) if p[3] else (255, 255, 255))
    ihdr = struct.pack(">IIBBBBB", W, H, 8, 6 if alpha else 2, 0, 0, 0)
    phys = struct.pack(">IIB", 11811, 11811, 1)          # 300 dpi
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"pHYs", phys)
    data += chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    path.write_bytes(data)


def heic(path):
    import sys
    import tempfile
    import numpy as np
    sys.path.insert(0, str(HERE.parents[3] / "tools"))
    import heic_check as hc
    px = np.array([[pixel(x, y) for x in range(W)] for y in range(H)], np.uint16)
    planes = [px[..., 1], px[..., 2], px[..., 0]]          # identity: Y = G, U = B, V = R
    with tempfile.TemporaryDirectory() as td:
        st = hc.make_stream(pathlib.Path(td), W, H, 3, 8, params="lossless=1", planes=planes)
        al = hc.make_stream(pathlib.Path(td), W, H, 0, 8, params="lossless=1", planes=[px[..., 3]])
    h, cid = hc.single(st, cicp_box=hc.nclx(1, 13, 0, 1), extra_props=[hc.box(b"irot", b"\x02")])
    aid = h.item(b"hvc1", hc.coded_data(al))
    h.prop(hc.hvcc(al), aid, True)
    h.prop(hc.ispe(W, H), aid)
    h.prop(hc.fbox(b"auxC", 0, 0, hc.URN_HEVC + b"\0"), aid, True)
    h.ref(b"auxl", aid, [cid])
    path.write_bytes(h.build(cid))


def avif(path):
    """a1_rgba as lossless 4:4:4 AV1 (identity matrix, sRGB: full range) with a lossless monochrome
    alpha item, rotated by 180 degrees through irot (like a1_rgba.heic)"""
    import sys
    import tempfile
    import numpy as np
    sys.path.insert(0, str(HERE.parents[3] / "tools"))
    import heic_check as hc
    import avif_check as ac
    px = np.array([[pixel(x, y) for x in range(W)] for y in range(H)], np.uint16)
    planes = [px[..., 1], px[..., 2], px[..., 0]]          # identity: Y = G, U = B, V = R
    with tempfile.TemporaryDirectory() as td:
        st = ac.make_av1(pathlib.Path(td), W, H, 3, 8, "aom", "-crf 0 -lossless 1 -cpu-used 6 -colorspace rgb "
                         "-color_primaries bt709 -color_trc iec61966-2-1", planes=planes)
        al = ac.make_av1(pathlib.Path(td), W, H, 0, 8, "aom", "-crf 0 -lossless 1 -cpu-used 6",
                         planes=[px[..., 3]])
    h, cid = ac.single(st, extra_props=[hc.box(b"irot", b"\x02")])
    aid = h.item(b"av01", al.item)
    h.prop(al.av1c(), aid, True)
    h.prop(hc.ispe(W, H), aid)
    h.prop(hc.fbox(b"auxC", 0, 0, ac.URN_ALPHA + b"\0"), aid, True)
    h.ref(b"auxl", aid, [cid])
    path.write_bytes(ac.build(h, cid))


def jxl(path):
    """a1_rgba as lossless JPEG XL (modular, alpha extra channel)"""
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(HERE / "a1_rgba.png"), "-c:v", "libjxl",
                    "-distance", "0", "-effort", "7", str(path)], check=True)


if __name__ == "__main__":
    png(HERE / "a1_rgba.png")
    tmp = HERE / "_opaque.png"
    png(tmp, alpha=False)
    for out, extra in (("a1_rgb.jpg", ["-q:v", "2"]), ("a1_rgb.bmp", []), ("a1_pal.gif", [])):
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(tmp)] + extra
                       + [str(HERE / out)], check=True)
    tmp.unlink()
    heic(HERE / "a1_rgba.heic")
    avif(HERE / "a1_rgba.avif")
    jxl(HERE / "a1_rgba.jxl")
