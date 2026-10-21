#!/usr/bin/env python3
"""Launch a Windows app, capture its client area to PNG, check its exit code.

Tests the real windowed path (window creation, WM_PAINT, SetDIBitsToDevice)
that --shot does not exercise.

  python tools/wincap.py build/win/template.exe --out build/cap.png [--delay 1.0]
         [--close-ms 2500] [-- extra app args]

The app is started with --auto-close so it quits by itself.  Exit code of this
script: 0 if the window was captured and the app exited with 0.
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import struct
import subprocess
import testenv  # noqa: F401  (keeps PaintCraft away from the user's settings)
import sys
import time
import zlib

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
try:
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass


def find_window(pid):
    found = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(hwnd, _):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid and user32.IsWindowVisible(hwnd):
            found.append(hwnd)
        return True
    user32.EnumWindows(cb, 0)
    return found[0] if found else None


def capture(hwnd):
    rc = wt.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(rc))
    w, h = rc.right, rc.bottom
    hdc = user32.GetDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    gdi32.SelectObject(mem, bmp)
    user32.PrintWindow(hwnd, mem, 1 | 2)  # PW_CLIENTONLY | PW_RENDERFULLCONTENT
    bmi = struct.pack("<IiiHHIIiiII", 40, w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, bmi, 0)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    return w, h, buf.raw


def write_png(path, w, h, bgra):
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        row = bgra[y * w * 4:(y + 1) * w * 4]
        for x in range(w):
            b, g, r = row[x * 4], row[x * 4 + 1], row[x * 4 + 2]
            raw += bytes((r, g, b))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 6)) + chunk(b"IEND", b"")
    open(path, "wb").write(png)


def main():
    argv = sys.argv[1:]
    extra = []
    if "--" in argv:
        i = argv.index("--")
        argv, extra = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("--out", default="build/cap.png")
    ap.add_argument("--delay", type=float, default=1.0)
    ap.add_argument("--close-ms", type=int, default=3000)
    a = ap.parse_args(argv)
    import os
    proc = subprocess.Popen([os.path.abspath(a.exe), "--auto-close", str(a.close_ms)] + extra)
    time.sleep(a.delay)
    hwnd = None
    for _ in range(50):
        hwnd = find_window(proc.pid)
        if hwnd:
            break
        time.sleep(0.1)
    ok = False
    if hwnd:
        w, h, px = capture(hwnd)
        write_png(a.out, w, h, px)
        print(f"captured {w}x{h} -> {a.out}")
        ok = True
    else:
        print("no window found")
    try:
        rc = proc.wait(timeout=a.close_ms / 1000 + 10)
    except subprocess.TimeoutExpired:
        proc.kill()
        print("app did not exit (killed)")
        return 1
    print("app exit code:", rc)
    return 0 if ok and rc == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
