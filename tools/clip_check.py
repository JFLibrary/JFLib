#!/usr/bin/env python3
"""Checks of the image clipboard (CLIP_IMAGE) and of the DIB / BMP conversions (agent K1).

  python tools/clip_check.py [--target win|linux|both] [--out build/k1] [--no-build]
      Headless only (the in-process clipboard of --script runs; the real clipboard is never
      touched), PaintCraft on each target:
      * DIB variants through `clip dib F` (the CF_DIB / CF_DIBV5 path of Windows,
        clip_dib_to_bmp): BITMAPCOREHEADER / INFOHEADER / V4 / V5, bottom-up and top-down,
        32 bit BI_RGB / BI_BITFIELDS (masks after a 40-byte header or inside V4 / V5) /
        BI_ALPHABITFIELDS, 24 bit (row padding), 8 bit palette, paint.net's alpha rule (a
        32-bit DIB whose alpha is 0 everywhere is opaque); every pixel is decoded by
        img_decode_ex in the app (test hook 172) and compared;
      * BMP files through `clip file F` (the image/bmp path of Wayland / X11, clip_bmp_fix);
      * invalid headers (zero / huge sizes, truncated pixels, bad depth, huge colour table):
        `clip dib` must refuse them (no crash);
      * the copy side: an image with alpha is imported as a layer (drop), selected and copied
        (Kopieren); the PNG and the BMP (V5, 32 bit, BI_BITFIELDS, bottom-up, straight alpha)
        of the clipboard are saved (`clip save`) and decoded here, and the BMP goes back
        through the image/bmp path.
  python tools/clip_check.py real-win [--out build/k1] [--no-build]
      The REAL Windows clipboard (its text is saved before and put back afterwards; other
      clipboard formats of the user are lost) and a real window (--auto-close):
      * a 32-bit BI_RGB DIB whose alpha is 0 everywhere is put on the clipboard as CF_DIB
        only; PaintCraft's toolbar buttons Einfügen and Kopieren are clicked by posted mouse
        messages (never SendInput); the clipboard then must hold "PNG", CF_DIBV5 (V5 header,
        BI_BITFIELDS, 32 bit) and CF_DIB (40-byte header, BI_RGB) with the opaque pixels
        (paint.net's alpha rule), all three decoded here;
      * jfgui_TEST_DROPFILES: the app posts itself a WM_DROPFILES with a real HDROP of two
        paths: the event log must show E_DROP (2 files at 100, 100) and the "Drag & Drop"
        question window must appear.
  python tools/clip_check.py real-x11 [--out build/k1] [--no-build]
      PaintCraft on WSLg's X11 (a real window, --auto-close) with a private selection
      (jfgui_X11SELECTION, so the Windows clipboard is not touched): a Python X client owns
      it with image/bmp only (alpha 0 everywhere); Einfügen and Kopieren are clicked (posted
      mouse messages to the WSLg window); the client then reads image/png and image/bmp from
      the app.  XDND: the client drags two file:// URIs (one with localhost and %XX) onto
      the window: XdndAware 5, XdndStatus (accept, copy), the uri-list transfer, XdndFinished
      (success), E_DROP in the event log and the "Drag & Drop" window.
  python tools/clip_check.py real-wl [--out build/k1] [--no-build]
      PaintCraft on WSLg's Wayland: the REAL Windows clipboard (text saved and put back) holds
      a 32-bit DIB with alpha 0; WSLg's bridge offers it to Wayland clients; Einfügen and
      Kopieren are clicked; the image must come back to the Windows clipboard ("PNG" or
      CF_DIB through the bridge) with the opaque pixels.  (Wayland drag and drop needs a
      drag source with a real pointer grab: not tested here.)
Exit code 0 = all checks passed.  Work files: <out>/clipchk/.
"""
import argparse
import os
import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402

W, H = 7, 5
FAILS = []
CFG = "build/k1/cfg"            # real windows: PaintCraft's settings.ini goes here, never to the user's


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else "  " + detail))
    if not ok:
        FAILS.append(name)
    return ok


# ------------------------------------------------------------------ source pixels (straight ARGB)
def source(alphas=(255, 128, 0, 64, 255, 1, 200)):
    px = []
    for y in range(H):
        row = []
        for x in range(W):
            a = alphas[(x + y) % len(alphas)]
            r = (x * 37 + y * 11 + 5) & 255
            g = (x * 5 + y * 71 + 9) & 255
            b = (x * 91 + y * 3 + 17) & 255
            row.append((a, r, g, b))
        px.append(row)
    return px


def rows32(px, topdown, order="bgra"):
    out = []
    for y in (range(H) if topdown else range(H - 1, -1, -1)):
        for (a, r, g, b) in px[y]:
            out.append(bytes([b, g, r, a]))
    return b"".join(out)


def rows24(px, topdown):
    stride = (W * 3 + 3) & ~3
    out = b""
    for y in (range(H) if topdown else range(H - 1, -1, -1)):
        row = b"".join(bytes([b, g, r]) for (a, r, g, b) in px[y])
        out += row + b"\0" * (stride - len(row))
    return out


def info_header(bpp, comp, topdown, hsz=40, masks=None, clrused=0, sizeimage=0, w=W, h=H):
    hh = -h if topdown else h
    d = struct.pack("<IiiHHIIiiII", hsz, w, hh, 1, bpp, comp, sizeimage, 2835, 2835, clrused, 0)
    if hsz >= 108:
        m = masks or (0, 0, 0, 0)
        d += struct.pack("<IIII", *m) + b"BGRs" + b"\0" * 36 + b"\0" * 12
    if hsz == 124:
        d += struct.pack("<IIII", 4, 0, 0, 0)
    if hsz == 40 and comp == 3:
        d += struct.pack("<III", *(masks or (0xFF0000, 0xFF00, 0xFF))[:3])
    if hsz == 40 and comp == 6:
        d += struct.pack("<IIII", *(masks or (0xFF0000, 0xFF00, 0xFF, 0xFF000000)))
    return d


def opaque(px):
    return [[(255, r, g, b) for (a, r, g, b) in row] for row in px]


def with_alpha(px, a):
    return [[(a, r, g, b) for (_, r, g, b) in row] for row in px]


STD = (0xFF0000, 0xFF00, 0xFF, 0xFF000000)


def dib_variants():
    s = source()
    z = with_alpha(s, 0)
    v = []
    v.append(("info32_rgb_bu", info_header(32, 0, False) + rows32(s, False), s))
    v.append(("info32_rgb_td", info_header(32, 0, True) + rows32(s, True), s))
    v.append(("info32_rgb_alpha0", info_header(32, 0, False) + rows32(z, False), opaque(s)))
    v.append(("info32_bf_noalpha", info_header(32, 3, False) + rows32(s, False), opaque(s)))
    v.append(("v5_bf_alpha_bu", info_header(32, 3, False, 124, STD) + rows32(s, False), s))
    v.append(("v5_bf_alpha_td", info_header(32, 3, True, 124, STD) + rows32(s, True), s))
    v.append(("v5_bf_alpha0", info_header(32, 3, False, 124, STD) + rows32(z, False), opaque(s)))
    v.append(("v4_bf_alpha_td", info_header(32, 3, True, 108, STD) + rows32(s, True), s))
    v.append(("info32_alphabf", info_header(32, 6, False) + rows32(s, False), s))
    v.append(("v5_rgb_alpha", info_header(32, 0, False, 124) + rows32(s, False), s))
    v.append(("info24_bu", info_header(24, 0, False) + rows24(s, False), opaque(s)))
    v.append(("info24_td", info_header(24, 0, True) + rows24(s, True), opaque(s)))
    v.append(("v5_24_bu", info_header(24, 0, False, 124) + rows24(s, False), opaque(s)))
    # 8-bit palette (clrUsed 0 = 256 entries), indices x + y * W
    pal = b""
    ppx = []
    for i in range(256):
        pal += bytes([(i * 7) & 255, (i * 13) & 255, (i * 29) & 255, 0])
    stride = (W + 3) & ~3
    rows = b""
    for y in range(H - 1, -1, -1):
        rows += bytes([x + y * W for x in range(W)]) + b"\0" * (stride - W)
    for y in range(H):
        ppx.append([(255, ((x + y * W) * 29) & 255, ((x + y * W) * 13) & 255, ((x + y * W) * 7) & 255)
                    for x in range(W)])
    v.append(("info8_pal", info_header(8, 0, False) + pal + rows, ppx))
    # BITMAPCOREHEADER, 24 bit
    core = struct.pack("<IHHHH", 12, W, H, 1, 24)
    v.append(("core24", core + rows24(s, False), opaque(s)))
    return v


def invalid_variants():
    s = source()
    good = info_header(32, 0, False) + rows32(s, False)
    v = []
    v.append(("inv_trunc_pixels", good[:-5]))
    v.append(("inv_width0", info_header(32, 0, False, w=0) + rows32(s, False)))
    v.append(("inv_height0", info_header(32, 0, False, h=0) + rows32(s, False)))
    v.append(("inv_huge", info_header(32, 0, False, w=60000, h=60000) + rows32(s, False)))
    v.append(("inv_hsz_big", struct.pack("<I", 5000) + good[4:]))
    v.append(("inv_hsz_small", struct.pack("<I", 20) + good[4:]))
    v.append(("inv_bpp7", info_header(7, 0, False) + rows32(s, False)))
    v.append(("inv_clrused", info_header(24, 0, False, clrused=0x40000000) + rows24(s, False)))
    v.append(("inv_tiny", good[:10]))
    v.append(("inv_hmin", info_header(32, 0, False, h=-0x80000000) + rows32(s, False)))
    return v


def bmp_file(dib, off):
    return b"BM" + struct.pack("<IHHI", 14 + len(dib), 0, 0, 14 + off) + dib


# ------------------------------------------------------------------ PNG (stdlib)
def png_encode(px):
    raw = b""
    for row in px:
        raw += b"\0" + b"".join(bytes([r, g, b, a]) for (a, r, g, b) in row)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", len(px[0]), len(px), 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def png_decode(b):
    assert b[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat = 8, b""
    while pos < len(b):
        n, t = struct.unpack(">I4s", b[pos:pos + 8])
        d = b[pos + 8:pos + 8 + n]
        pos += 12 + n
        if t == b"IHDR":
            w, h, bd, ct = struct.unpack(">IIBB", d[:10])
        elif t == b"IDAT":
            idat += d
    assert bd == 8 and ct in (2, 6)
    ch = 4 if ct == 6 else 3
    raw = zlib.decompress(idat)
    stride = w * ch
    prev = bytearray(stride)
    out, i = [], 0
    for y in range(h):
        f = raw[i]
        cur = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = cur[x - ch] if x >= ch else 0
            c = prev[x - ch] if x >= ch else 0
            u = prev[x]
            if f == 1:
                cur[x] = (cur[x] + a) & 255
            elif f == 2:
                cur[x] = (cur[x] + u) & 255
            elif f == 3:
                cur[x] = (cur[x] + ((a + u) >> 1)) & 255
            elif f == 4:
                p = a + u - c
                pa, pb, pc = abs(p - a), abs(p - u), abs(p - c)
                cur[x] = (cur[x] + (a if pa <= pb and pa <= pc else (u if pb <= pc else c))) & 255
        out.append([(cur[x * ch + 3] if ch == 4 else 255, cur[x * ch], cur[x * ch + 1], cur[x * ch + 2])
                    for x in range(w)])
        prev = cur
    return w, h, out


# ------------------------------------------------------------------ runs
def run_script(target, out, body, name):
    d = out / "clipchk"
    d.mkdir(parents=True, exist_ok=True)
    p = d / (name + ".uis")
    p.write_text(body, encoding="utf-8", newline="\n")
    exe = build.exe_path("PaintCraft", target, out)
    cmd = build.run_cmd_for(exe, target, ["--script", str(p.relative_to(ROOT)).replace("\\", "/")])
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=600)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def rel(p):
    return str(p.relative_to(ROOT)).replace("\\", "/")


def probe_lines(px):
    lines = []
    for y in range(len(px)):
        for x in range(len(px[0])):
            a, r, g, b = px[y][x]
            lines.append("cmd 172 %d" % (x | y << 16))
            lines.append('expect #0x82200 text "%d,%d,%02X%02X%02X%02X"' % (len(px[0]), len(px), a, r, g, b))
    return lines


def check_target(target, out):
    d = out / "clipchk"
    d.mkdir(parents=True, exist_ok=True)
    # ---- valid DIBs and BMP files, one script
    body = ["size 1280 800"]
    names = []
    for name, dib, exp in dib_variants():
        f = d / (name + ".dib")
        f.write_bytes(dib)
        body.append("print CASE " + name)
        body.append("clip dib " + rel(f))
        body += probe_lines(exp)
        names.append(name)
    s = source()
    z = with_alpha(s, 0)
    files = [("bmp_v5_alpha", bmp_file(info_header(32, 3, False, 124, STD) + rows32(s, False), 124), s),
             ("bmp_v5_alpha0", bmp_file(info_header(32, 3, False, 124, STD) + rows32(z, False), 124), opaque(s)),
             ("bmp_info32_rgb", bmp_file(info_header(32, 0, True) + rows32(s, True), 40), s),
             ("bmp_24", bmp_file(info_header(24, 0, False) + rows24(s, False), 40), opaque(s))]
    for name, data, exp in files:
        f = d / (name + ".bmp")
        f.write_bytes(data)
        body.append("print CASE " + name)
        body.append("clip file " + rel(f))
        body += probe_lines(exp)
        names.append(name)
    rc, txt = run_script(target, out, "\n".join(body) + "\n", "valid_" + target)
    cur = None
    bad = {}
    for line in txt.splitlines():
        if line.startswith("CASE "):
            cur = line[5:]
        elif line.startswith("FAIL") and cur:
            bad.setdefault(cur, line)
    for n in names:
        check(n not in bad and "crash" not in txt, "[%s] %s" % (target, n), bad.get(n, txt[-300:]))
    # ---- invalid DIBs: refused, no crash
    for name, dib in invalid_variants():
        f = d / (name + ".dib")
        f.write_bytes(dib)
        rc, txt = run_script(target, out, "size 800 600\nclip dib %s\nclip expect none\n" % rel(f), name)
        check(rc == 1 and "invalid DIB" in txt and "crash" not in txt and txt.count("FAIL") == 1,
              "[%s] %s refused" % (target, name), txt.strip()[-200:])
    # ---- copy side: import an image with alpha as a layer, select it, Kopieren
    src = source((255, 128, 0, 64, 255, 32, 200))
    sp = d / "copy_src.png"
    sp.write_bytes(png_encode(src))
    op, ob = d / ("copy_out_%s.png" % target), d / ("copy_out_%s.bmp" % target)
    for p in (op, ob):
        if p.exists():
            p.unlink()
    body = ["size 1280 800", "theme light", "clip clear",
            "drop 600 400 " + rel(sp), 'click "Ebene hinzufügen"', "cmd 750",
            "drag 240,148 %d,%d" % (240 + W, 148 + H), "key ctrl+c", "clip expect image",
            "clip save " + rel(op), "clip save %s bmp" % rel(ob), "clip file " + rel(ob)]
    body += probe_lines(src)
    rc, txt = run_script(target, out, "\n".join(body) + "\n", "copy_" + target)
    check(rc == 0, "[%s] copy script" % target, txt.strip()[-400:])
    if op.exists():
        w, h, px = png_decode(op.read_bytes())
        ok = (w, h) == (W, H) and all(px[y][x][0] == src[y][x][0] and (src[y][x][0] == 0 or px[y][x] == src[y][x])
                                      for y in range(H) for x in range(W))
        check(ok, "[%s] copied PNG = source" % target, "%dx%d" % (w, h))
    else:
        check(False, "[%s] copied PNG saved" % target)
    if ob.exists():
        b = ob.read_bytes()
        off, = struct.unpack("<I", b[10:14])
        hsz, w, h, pl, bpp, comp = struct.unpack("<IiiHHI", b[14:34])
        masks = struct.unpack("<IIII", b[54:70])
        ok = (b[:2] == b"BM" and hsz == 124 and (w, h) == (W, H) and bpp == 32 and comp == 3
              and masks == STD and off == 138 and len(b) == 138 + W * H * 4)
        check(ok, "[%s] copied BMP header (V5, 32 bit, BI_BITFIELDS, bottom-up)" % target,
              "%s %d %d %d %d %d %s %d" % (b[:2], hsz, w, h, bpp, comp, masks, off))
        if ok:
            good = True
            for y in range(H):
                row = b[off + (H - 1 - y) * W * 4: off + (H - y) * W * 4]
                for x in range(W):
                    bb, g, r, a = row[x * 4:x * 4 + 4]
                    e = src[y][x]
                    if a != e[0] or (a and (a, r, g, bb) != e):
                        good = False
            check(good, "[%s] copied BMP pixels = source (straight alpha)" % target)
    else:
        check(False, "[%s] copied BMP saved" % target)


# ------------------------------------------------------------------ real Windows clipboard / drop
def real_win(out):
    import ctypes
    import ctypes.wintypes as wt
    import time
    user32, kernel32 = ctypes.windll.user32, ctypes.windll.kernel32
    try:
        user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    except Exception:
        pass
    user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
    user32.GetClipboardData.restype = ctypes.c_void_p
    user32.SetClipboardData.argtypes = [wt.UINT, ctypes.c_void_p]
    user32.SetClipboardData.restype = ctypes.c_void_p
    user32.RegisterClipboardFormatW.restype = wt.UINT
    kernel32.GlobalAlloc.restype = ctypes.c_void_p
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalSize.restype = ctypes.c_size_t
    kernel32.GlobalSize.argtypes = [ctypes.c_void_p]
    CF_UNICODETEXT, CF_DIB, CF_DIBV5 = 13, 8, 17
    cf_png = user32.RegisterClipboardFormatW("PNG")
    exe = str(build.exe_path("PaintCraft", "win", out))

    def open_cb():
        for _ in range(50):
            if user32.OpenClipboard(None):
                return True
            time.sleep(0.02)
        return False

    def get(fmt):
        if not user32.IsClipboardFormatAvailable(fmt) or not open_cb():
            return None
        try:
            h = user32.GetClipboardData(fmt)
            if not h:
                return None
            p = kernel32.GlobalLock(h)
            n = kernel32.GlobalSize(h)
            data = ctypes.string_at(p, n)
            kernel32.GlobalUnlock(h)
            return data
        finally:
            user32.CloseClipboard()

    def put(fmt, data):
        h = kernel32.GlobalAlloc(2, len(data))
        p = kernel32.GlobalLock(h)
        ctypes.memmove(p, data, len(data))
        kernel32.GlobalUnlock(h)
        if not open_cb():
            return False
        user32.EmptyClipboard()
        ok = bool(user32.SetClipboardData(fmt, h))
        user32.CloseClipboard()
        return ok

    def windows_of(pid):
        res = []

        @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
        def cb(h, _):
            p = wt.DWORD()
            user32.GetWindowThreadProcessId(h, ctypes.byref(p))
            if p.value == pid and user32.IsWindowVisible(h):
                n = user32.GetWindowTextLengthW(h)
                b = ctypes.create_unicode_buffer(n + 1)
                user32.GetWindowTextW(h, b, n + 1)
                res.append((h, b.value))
            return True
        user32.EnumWindows(cb, 0)
        return res

    def click(h, x, y):
        lp = ((y & 0xFFFF) << 16) | (x & 0xFFFF)
        user32.PostMessageW(h, 0x200, 0, lp)
        user32.PostMessageW(h, 0x201, 1, lp)
        user32.PostMessageW(h, 0x202, 0, lp)
        time.sleep(0.8)

    saved = get(CF_UNICODETEXT)
    print("real clipboard: text saved (%s)" % ("%d bytes" % len(saved) if saved else "none"))
    d = out / "clipchk"
    d.mkdir(parents=True, exist_ok=True)
    s = source()
    z = with_alpha(s, 0)
    try:
        # ---- CF_DIB in, "PNG" + CF_DIBV5 + CF_DIB out
        check(put(CF_DIB, info_header(32, 0, False) + rows32(z, False)), "real: CF_DIB put on the clipboard")
        log = d / "real_win.log"
        proc = subprocess.Popen([exe, "--size", "1280x800", "--auto-close", "20000", "--log-events", "--config", CFG],
                                stdout=open(log, "w"), stderr=subprocess.STDOUT, cwd=ROOT)
        hwnd = None
        for _ in range(100):
            w = [h for h, t in windows_of(proc.pid) if t.endswith("PaintCraft")]
            if w:
                hwnd = w[0]
                break
            time.sleep(0.1)
        check(hwnd is not None, "real: PaintCraft window")
        if hwnd:
            time.sleep(1.0)
            dpi = user32.GetDpiForWindow(hwnd) or 96
            sc = dpi / 96.0
            click(hwnd, int(258 * sc), int(68 * sc))            # toolbar Einfügen (#163)
            click(hwnd, int(224 * sc), int(68 * sc))            # toolbar Kopieren (#161)
            time.sleep(1.0)
            png = get(cf_png)
            v5 = get(CF_DIBV5)
            dib = get(CF_DIB)
            exp = opaque(s)
            ok = png is not None
            if ok:
                w, h, px = png_decode(png)
                ok = (w, h) == (W, H) and px == exp
            check(ok, 'real: "PNG" written by Kopieren = the pasted DIB, opaque (alpha rule)')
            for name, blob, hsz, comp in (("CF_DIBV5", v5, 124, 3), ("CF_DIB", dib, 40, 0)):
                ok = blob is not None and len(blob) >= hsz
                if ok:
                    size, w, h, pl, bpp, cp = struct.unpack("<IiiHHI", blob[:20])
                    ok = (size, w, h, bpp, cp) == (hsz, W, H, 32, comp)
                    if ok:
                        pxs = blob[hsz:hsz + W * H * 4]
                        for y in range(H):
                            for x in range(W):
                                i = ((H - 1 - y) * W + x) * 4
                                bb, g, r, a = pxs[i:i + 4]
                                if (a, r, g, bb) != exp[y][x]:
                                    ok = False
                check(ok, "real: %s written by Kopieren (header, pixels)" % name)
            user32.PostMessageW(hwnd, 0x10, 0, 0)               # WM_CLOSE: asks (modified)
        try:
            proc.wait(timeout=25)
        except subprocess.TimeoutExpired:
            proc.kill()
        # ---- WM_DROPFILES with a real HDROP (the app posts it to itself)
        env = dict(os.environ)
        env["jfgui_TEST_DROPFILES"] = ("apps/PaintCraft/tests/fixtures/a1_rgba.png|"
                                        "apps/PaintCraft/tests/fixtures/a1_rgb.bmp")
        log2 = d / "real_win_drop.log"
        proc = subprocess.Popen([exe, "--size", "1280x800", "--auto-close", "6000", "--log-events", "--config", CFG],
                                stdout=open(log2, "w"), stderr=subprocess.STDOUT, cwd=ROOT, env=env)
        seen = False
        for _ in range(50):
            if any(t == "Drag & Drop" for h, t in windows_of(proc.pid)):
                seen = True
                break
            time.sleep(0.1)
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
        txt = log2.read_text(encoding="utf-8", errors="replace")
        check("event type=17 btn=0 mods=0 x=100 y=100 data=2 win=0" in txt, "real: WM_DROPFILES -> E_DROP (2 files)")
        check(seen, 'real: the "Drag & Drop" question window appears')
    finally:
        if saved is not None:
            t = saved.decode("utf-16-le", "replace").split("\0")[0]
            put(CF_UNICODETEXT, (t + "\0").encode("utf-16-le"))
        elif open_cb():
            user32.EmptyClipboard()
            user32.CloseClipboard()
        print("real clipboard: text restored")


# ------------------------------------------------------------------ real X11 (WSLg): clipboard + XDND
XCLIENT = r'''
import os, socket, struct, sys, time
def pad(n): return (4 - n % 4) % 4
class X:
    def __init__(s):
        s.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.s.connect("/tmp/.X11-unix/X" + os.environ.get("DISPLAY", ":0").split(":")[1].split(".")[0])
        s.s.sendall(struct.pack("<BxHHHHxx", 0x6C, 11, 0, 0, 0))
        h = s.read(8); body = s.read(struct.unpack("<H", h[6:8])[0] * 4)
        s.base, s.mask = struct.unpack("<II", body[4:12]); vl = struct.unpack("<H", body[16:18])[0]
        nf = body[21]; off = 32 + vl + pad(vl) + 8 * nf
        s.root = struct.unpack("<I", body[off:off + 4])[0]
        s.seq = 0; s.next = 0; s.q = []
        s.win = s.id()
        s.req(struct.pack("<BBHIIhhHHHHII", 1, 0, 8 + 1, s.win, s.root, 0, 0, 1, 1, 0, 1, 0, 0x800) + struct.pack("<I", 0x400000))
    def read(s, n):
        b = b""
        while len(b) < n:
            c = s.s.recv(n - len(b))
            if not c: raise SystemExit("eof")
            b += c
        return b
    def id(s):
        s.next += 1; return s.base | s.next
    def req(s, d):
        s.s.sendall(d); s.seq += 1; return s.seq
    def packet(s):
        p = s.read(32)
        if p[0] == 1: p += s.read(struct.unpack("<I", p[4:8])[0] * 4)
        return p
    def reply(s, seq):
        while True:
            p = s.packet()
            if p[0] == 1 and struct.unpack("<H", p[2:4])[0] == seq & 0xFFFF: return p
            if p[0] == 0:
                if struct.unpack("<H", p[2:4])[0] == seq & 0xFFFF: return None
                continue
            s.q.append(p)
    def event(s, timeout=5):
        if s.q: return s.q.pop(0)
        s.s.settimeout(timeout)
        try: return s.packet()
        finally: s.s.settimeout(None)
    def atom(s, name):
        b = name.encode()
        return struct.unpack("<I", s.reply(s.req(struct.pack("<BBHHxx", 16, 0, 2 + (len(b) + pad(len(b))) // 4, len(b)) + b + b"\0" * pad(len(b))))[8:12])[0]
    def change(s, win, prop, typ, fmt, data, mode=0):
        n = len(data) // (fmt // 8)
        s.req(struct.pack("<BBHIIIBxxxI", 18, mode, 6 + (len(data) + pad(len(data))) // 4, win, prop, typ, fmt, n) + data + b"\0" * pad(len(data)))
    def getprop(s, win, prop, delete=0):
        r = s.reply(s.req(struct.pack("<BBHIIIII", 20, delete, 6, win, prop, 0, 0, 0x1000000)))
        if r is None: return 0, b""
        fmt = r[1]; typ, after, n = struct.unpack("<III", r[8:20])
        return typ, r[32:32 + n * fmt // 8]
    def children(s, w):
        r = s.reply(s.req(struct.pack("<BxHI", 15, 2, w)))
        if r is None: return []
        n = struct.unpack("<H", r[16:18])[0]
        return list(struct.unpack("<%dI" % n, r[32:32 + 4 * n]))
    def client_msg(s, dest, typ, l):
        ev = struct.pack("<BBHII5I", 33, 32, 0, dest, typ, *l)
        s.req(struct.pack("<BBHII", 25, 0, 11, dest, 0) + ev)
x = X()
mode = sys.argv[1]
SEL = x.atom(os.environ.get("PYX_SELECTION", "CLIPBOARD"))
TARGETS, PROP = x.atom("TARGETS"), x.atom("PYX_SEL")
if mode == "own":                      # own FILE MIME SECONDS: offer FILE as MIME
    data = open(sys.argv[2], "rb").read(); mime = x.atom(sys.argv[3])
    x.req(struct.pack("<BxHIII", 22, 4, x.win, SEL, 0))
    print("OWNER", flush=True)
    end = time.time() + float(sys.argv[4])
    while time.time() < end:
        try: e = x.event(1)
        except socket.timeout: continue
        if e[0] & 0x7F != 30: continue
        tm, owner, req, sel, target, prop = struct.unpack("<IIIIII", e[4:28])
        if target == TARGETS: x.change(req, prop, 4, 32, struct.pack("<II", TARGETS, mime))
        elif target == mime: x.change(req, prop, mime, 8, data); print("SENT", len(data), flush=True)
        else: prop = 0
        x.req(struct.pack("<BBHII", 25, 0, 11, req, 0) + struct.pack("<BxHIIIII8x", 31, 0, tm, req, sel, target, prop))
    sys.exit(0)
if mode == "get":                      # get TARGET OUTFILE
    target = x.atom(sys.argv[2])
    x.req(struct.pack("<BxHIIIII", 24, 6, x.win, SEL, target, PROP, 0))
    while True:
        e = x.event()
        if e[0] & 0x7F == 31: break
    if struct.unpack("<I", e[20:24])[0] == 0: print("REFUSED"); sys.exit(1)
    typ, data = x.getprop(x.win, PROP, 1)
    open(sys.argv[3], "wb").write(data)
    print("GOT", len(data), flush=True)
    sys.exit(0)
if mode == "dnd":                      # dnd TITLE URI...: drag the URIs onto the window TITLE
    WM_NAME, NET_NAME, UTF8 = 39, x.atom("_NET_WM_NAME"), x.atom("UTF8_STRING")
    AWARE, ENTER, POS, STATUS, DROP, FIN = (x.atom(n) for n in ("XdndAware", "XdndEnter", "XdndPosition",
                                                                "XdndStatus", "XdndDrop", "XdndFinished"))
    XSEL, COPY, URI = x.atom("XdndSelection"), x.atom("XdndActionCopy"), x.atom("text/uri-list")
    title = sys.argv[2].encode()
    target, todo = 0, [x.root]
    while todo and not target:
        w = todo.pop()
        for p in (NET_NAME, WM_NAME):
            t, v = x.getprop(w, p)
            if v.endswith(title):
                target = w
        todo += x.children(w)
    t, aware = x.getprop(target, AWARE) if target else (0, b"")
    print("TARGET %x aware %s" % (target, struct.unpack("<I", aware)[0] if len(aware) == 4 else "-"), flush=True)
    if not target: sys.exit(1)
    r = x.reply(x.req(struct.pack("<BxHIIhh", 40, 4, target, x.root, 100, 100)))
    rx, ry = struct.unpack("<hh", r[12:16])
    uris = ("".join(u + "\r\n" for u in sys.argv[3:])).encode()
    x.req(struct.pack("<BxHIII", 22, 4, x.win, XSEL, 0))
    x.client_msg(target, ENTER, [x.win, 5 << 24, URI, 0, 0])
    x.client_msg(target, POS, [x.win, 0, (rx << 16) | (ry & 0xFFFF), 0, COPY])
    end = time.time() + 10
    state = "wait-status"
    while time.time() < end:
        try: e = x.event(1)
        except socket.timeout: continue
        k = e[0] & 0x7F
        if k == 33:
            win, typ = struct.unpack("<II", e[4:12]); l = struct.unpack("<5I", e[12:32])
            if typ == STATUS and state == "wait-status":
                print("STATUS accept=%d action=%s" % (l[1] & 1, "copy" if l[4] == COPY else l[4]), flush=True)
                x.client_msg(target, DROP, [x.win, 0, 0, 0, 0]); state = "wait-finished"
            elif typ == FIN:
                print("FINISHED success=%d action=%s" % (l[1] & 1, "copy" if l[2] == COPY else l[2]), flush=True)
                break
        elif k == 30:
            tm, owner, req, sel, tgt, prop = struct.unpack("<IIIIII", e[4:28])
            if tgt == URI: x.change(req, prop, URI, 8, uris); print("SENT uri-list", flush=True)
            else: prop = 0
            x.req(struct.pack("<BBHII", 25, 0, 11, req, 0) + struct.pack("<BxHIIIII8x", 31, 0, tm, req, sel, tgt, prop))
    sys.exit(0)
'''


def real_x11(out):
    import time
    sys.path.insert(0, str(ROOT / "tools"))
    import x11check as xc
    distro = xc.DISTRO
    d = out / "clipchk"
    d.mkdir(parents=True, exist_ok=True)
    (d / "xclient.py").write_text(XCLIENT, encoding="utf-8", newline="\n")
    xcl = rel(d / "xclient.py")
    s = source()
    bmp = d / "x11_in.bmp"
    bmp.write_bytes(bmp_file(info_header(32, 3, False, 124, STD) + rows32(with_alpha(s, 0), False), 124))
    sel = "jfgui_TEST_SELECTION"                   # private selection: WSLg mirrors CLIPBOARD
    owner = subprocess.Popen(["wsl.exe", "-d", distro, "--cd", str(ROOT), "-e", "env", "PYX_SELECTION=" + sel,
                              "python3", xcl, "own", rel(bmp), "image/bmp", "30"],
                             stdout=subprocess.PIPE, text=True)
    time.sleep(1.0)
    exe = rel(Path(build.exe_path("PaintCraft", "linux", out)))
    app = xc.App(exe, ["--size", "1280x800", "--auto-close", "30000", "--log-events", "--config", CFG], "x11",
                 "PaintCraft (", env={"jfgui_X11SELECTION": sel}, log=rel(d / "real_x11.log"))
    hwnd = app.window()
    if not check(hwnd is not None, "x11: PaintCraft window"):
        app.kill()
        owner.kill()
        return
    w, h, px, pos = xc.wait_content(hwnd)
    time.sleep(1.0)
    w, h, px, pos = xc.wslcap.capture(hwnd, False)
    m = xc.margin_of(w, h, px) or 32
    cw, ch = xc.content_size(app, (w - 2 * m, h - 2 * m))
    if cw < w:
        m = (w - cw) // 2
    sc = cw / 1280.0
    xc.click(hwnd, m + int(258 * sc), m + int(68 * sc))    # toolbar Einfügen
    time.sleep(1.0)
    xc.click(hwnd, m + int(224 * sc), m + int(68 * sc))    # toolbar Kopieren
    time.sleep(1.5)
    exp = opaque(s)
    for mime, name in (("image/png", "x11_out.png"), ("image/bmp", "x11_out.bmp")):
        outf = d / name
        if outf.exists():
            outf.unlink()
        r = xc.wsl("timeout 10 env PYX_SELECTION=%s python3 %s get %s %s" % (sel, xcl, mime, rel(outf)))
        ok = r.stdout.startswith("GOT") and outf.exists()
        if ok and mime == "image/png":
            ww, hh, got = png_decode(outf.read_bytes())
            ok = (ww, hh) == (W, H) and got == exp
        elif ok:
            b = outf.read_bytes()
            off, = struct.unpack("<I", b[10:14])
            hsz, ww, hh, pl, bpp, comp = struct.unpack("<IiiHHI", b[14:34])
            ok = (hsz, ww, hh, bpp, comp, off) == (124, W, H, 32, 3, 138)
            for y in range(H):
                for xx in range(W):
                    i = off + ((H - 1 - y) * W + xx) * 4
                    bb, g, rr, a = b[i:i + 4]
                    ok = ok and (a, rr, g, bb) == exp[y][xx]
        check(ok, "x11: image/bmp pasted (alpha rule), Kopieren offers %s" % mime, r.stdout.strip()[:80])
    owner.kill()
    # XDND: a Python X client drags two file URIs onto the window
    root = str(ROOT).replace("\\", "/")
    wslroot = "/mnt/" + root[0].lower() + root[2:]
    uris = ["file://" + wslroot + "/apps/PaintCraft/tests/fixtures/a1_rgba.png",
            "file://localhost" + wslroot.replace(" ", "%20") + "/apps/PaintCraft/tests/fixtures/a1%5Frgb.bmp"]
    r = xc.wsl("timeout 20 python3 %s dnd ' PaintCraft' %s" % (xcl, " ".join(uris)))
    print(r.stdout.strip())
    check("aware 5" in r.stdout, "x11: XdndAware = 5 on the window")
    check("STATUS accept=1 action=copy" in r.stdout, "x11: XdndStatus accepts text/uri-list with copy")
    check("SENT uri-list" in r.stdout and "FINISHED success=1 action=copy" in r.stdout,
          "x11: the drop is read and finished (success, copy)")
    time.sleep(1.0)
    check("event type=17" in app.output() and "data=2 " in app.output(), "x11: E_DROP with 2 files in the event log")
    check(bool(xc.wslcap.windows_with_title("Drag & Drop")), 'x11: the "Drag & Drop" question window appears')
    app.kill()


def real_wl(out):
    """Wayland through WSLg's clipboard bridge (Windows clipboard <-> Wayland selection)."""
    import ctypes
    import ctypes.wintypes as wt
    import time
    sys.path.insert(0, str(ROOT / "tools"))
    import x11check as xc
    user32, kernel32 = ctypes.windll.user32, ctypes.windll.kernel32
    user32.GetClipboardData.restype = ctypes.c_void_p
    user32.SetClipboardData.argtypes = [wt.UINT, ctypes.c_void_p]
    user32.SetClipboardData.restype = ctypes.c_void_p
    user32.RegisterClipboardFormatW.restype = wt.UINT
    kernel32.GlobalAlloc.restype = ctypes.c_void_p
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalSize.restype = ctypes.c_size_t
    kernel32.GlobalSize.argtypes = [ctypes.c_void_p]
    CF_UNICODETEXT, CF_DIB = 13, 8
    cf_png = user32.RegisterClipboardFormatW("PNG")

    def open_cb():
        for _ in range(50):
            if user32.OpenClipboard(None):
                return True
            time.sleep(0.02)
        return False

    def get(fmt):
        if not user32.IsClipboardFormatAvailable(fmt) or not open_cb():
            return None
        try:
            h = user32.GetClipboardData(fmt)
            if not h:
                return None
            p = kernel32.GlobalLock(h)
            data = ctypes.string_at(p, kernel32.GlobalSize(h))
            kernel32.GlobalUnlock(h)
            return data
        finally:
            user32.CloseClipboard()

    def put(fmt, data):
        h = kernel32.GlobalAlloc(2, len(data))
        p = kernel32.GlobalLock(h)
        ctypes.memmove(p, data, len(data))
        kernel32.GlobalUnlock(h)
        if not open_cb():
            return False
        user32.EmptyClipboard()
        ok = bool(user32.SetClipboardData(fmt, h))
        user32.CloseClipboard()
        return ok

    saved = get(CF_UNICODETEXT)
    print("real clipboard: text saved (%s)" % ("%d bytes" % len(saved) if saved else "none"))
    d = out / "clipchk"
    d.mkdir(parents=True, exist_ok=True)
    s = source()
    exp = opaque(s)
    try:
        check(put(CF_DIB, info_header(32, 0, False) + rows32(with_alpha(s, 0), False)),
              "wayland: CF_DIB put on the Windows clipboard (WSLg offers it to Wayland)")
        time.sleep(1.5)
        exe = rel(Path(build.exe_path("PaintCraft", "linux", out)))
        app = xc.App(exe, ["--size", "1280x800", "--auto-close", "30000", "--log-events", "--config", CFG],
                     "wayland", "PaintCraft (", log=rel(d / "real_wl.log"))
        hwnd = app.window()
        if check(hwnd is not None, "wayland: PaintCraft window"):
            xc.wait_content(hwnd)
            time.sleep(1.0)
            w, h, px, pos = xc.wslcap.capture(hwnd, False)
            m = xc.margin_of(w, h, px)
            cw, ch = xc.content_size(app, (w - 2 * m, h - 2 * m))
            if cw < w:
                m = (w - cw) // 2
            sc = cw / 1280.0
            before = get(cf_png)
            xc.click(hwnd, m + int(258 * sc), m + int(68 * sc))    # toolbar Einfügen
            time.sleep(1.0)
            xc.click(hwnd, m + int(224 * sc), m + int(68 * sc))    # toolbar Kopieren
            time.sleep(2.5)
            png, dib = get(cf_png), get(CF_DIB)
            got = None
            if png and png != before:
                ww, hh, got = png_decode(png)
                got = got if (ww, hh) == (W, H) else None
            elif dib:
                hsz, ww, hh, pl, bpp, comp = struct.unpack("<IiiHHI", dib[:20])
                if (ww, abs(hh), bpp) == (W, H, 32):
                    off = hsz + (12 if hsz == 40 and comp == 3 else 0)
                    got = []
                    for y in range(H):
                        yy = (H - 1 - y) if hh > 0 else y
                        row = []
                        for x in range(W):
                            bb, g, r, a = dib[off + (yy * W + x) * 4: off + (yy * W + x) * 4 + 4]
                            row.append((255 if comp == 0 else a, r, g, bb))
                        got.append(row)
            check(got == exp, "wayland: Einfügen of the bridged image (alpha rule), Kopieren back to Windows",
                  "PNG %s, CF_DIB %s" % (png is not None, dib is not None))
        app.kill()
    finally:
        if saved is not None:
            t = saved.decode("utf-16-le", "replace").split("\0")[0]
            put(CF_UNICODETEXT, (t + "\0").encode("utf-16-le"))
        elif open_cb():
            user32.EmptyClipboard()
            user32.CloseClipboard()
        print("real clipboard: text restored")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", nargs="?", default="headless", choices=["headless", "real-win", "real-x11", "real-wl"])
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/k1")
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    build.OUT_DIR = out
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    if a.mode == "real-win":
        if not a.no_build:
            build.build_lib("win", out)
            build.build_app("PaintCraft", "win", out)
        real_win(out)
        targets = []
    if a.mode == "real-x11":
        if not a.no_build:
            build.build_lib("linux", out)
            build.build_app("PaintCraft", "linux", out)
        real_x11(out)
        targets = []
    if a.mode == "real-wl":
        if not a.no_build:
            build.build_lib("linux", out)
            build.build_app("PaintCraft", "linux", out)
        real_wl(out)
        targets = []
    for t in targets:
        if not a.no_build:
            build.build_lib(t, out)
            build.build_app("PaintCraft", t, out)
        check_target(t, out)
    print("clip_check:", "all passed" if not FAILS else "%d failed" % len(FAILS))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
