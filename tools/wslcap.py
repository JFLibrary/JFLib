#!/usr/bin/env python3
"""Launch a Linux build through WSL and capture its WSLg window from Windows.

WSLg shows Wayland toplevels as RDP RAIL windows owned by msrdc.exe whose
title is the xdg_toplevel title, so the window is found by title.

  python tools/wslcap.py --title "jfgui Template" --out build/cap_linux.png \
         [--delay 1.5] [--close-ms 4000] [--screen] -- build/linux/template [app args]

The app gets --auto-close CLOSE_MS.  --screen copies the window area from the
screen (BitBlt) instead of PrintWindow.  Exit code 0 if the window was captured
and the app exited with 0.
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import os
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

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DISTRO = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")


def windows_with_title(title):
    found = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(hwnd, _):
        n = user32.GetWindowTextLengthW(hwnd)
        if n and user32.IsWindowVisible(hwnd):
            buf = ctypes.create_unicode_buffer(n + 1)
            user32.GetWindowTextW(hwnd, buf, n + 1)
            if title in buf.value:
                cls = ctypes.create_unicode_buffer(256)
                user32.GetClassNameW(hwnd, cls, 256)
                found.append((hwnd, buf.value, cls.value))
        return True
    user32.EnumWindows(cb, 0)
    return found


def capture(hwnd, screen):
    rc = wt.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(rc))
    w, h = rc.right, rc.bottom
    pt = wt.POINT(0, 0)
    user32.ClientToScreen(hwnd, ctypes.byref(pt))
    hdc_src = user32.GetDC(None if screen else hwnd)
    mem = gdi32.CreateCompatibleDC(hdc_src)
    bmp = gdi32.CreateCompatibleBitmap(hdc_src, w, h)
    gdi32.SelectObject(mem, bmp)
    if screen:
        gdi32.BitBlt(mem, 0, 0, w, h, hdc_src, pt.x, pt.y, 0x00CC0020)
    else:
        user32.PrintWindow(hwnd, mem, 1 | 2)
    bmi = struct.pack("<IiiHHIIiiII", 40, w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, bmi, 0)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(None if screen else hwnd, hdc_src)
    if not screen and w and h and not any(buf.raw):
        # WSLg sometimes answers PrintWindow with an all-black image (e.g. after its
        # compositor restarted): copy the window's area from the screen instead
        return capture(hwnd, True)
    return w, h, buf.raw, (pt.x, pt.y)


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
    cmd = []
    if "--" in argv:
        i = argv.index("--")
        argv, cmd = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("--title", required=True)
    ap.add_argument("--out", default="build/cap_linux.png")
    ap.add_argument("--delay", type=float, default=1.5)
    ap.add_argument("--close-ms", type=int, default=4000)
    ap.add_argument("--screen", action="store_true")
    a = ap.parse_args(argv)
    if not cmd:
        ap.error("missing -- EXE [args]")
    exe = cmd[0].replace("\\", "/")
    if not exe.startswith("/"):
        exe = "./" + exe
    full = ["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", exe, "--auto-close", str(a.close_ms)] + cmd[1:]
    proc = subprocess.Popen(full, cwd=ROOT)
    time.sleep(a.delay)
    wins = []
    for _ in range(50):
        wins = windows_with_title(a.title)
        if wins:
            break
        time.sleep(0.1)
    ok = False
    if wins:
        hwnd, title, cls = wins[0]
        if a.screen:
            user32.SetForegroundWindow(hwnd)
            time.sleep(0.3)
        w, h, px, pos = capture(hwnd, a.screen)
        write_png(a.out, w, h, px)
        r = wt.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        print(f"window {title!r} class {cls!r}: client {w}x{h} at {pos}, "
              f"frame {r.right - r.left}x{r.bottom - r.top} -> {a.out}")
        ok = True
    else:
        print("no window found")
    try:
        rc = proc.wait(timeout=a.close_ms / 1000 + 15)
    except subprocess.TimeoutExpired:
        proc.kill()
        print("app did not exit (killed)")
        return 1
    print("app exit code:", rc)
    return 0 if ok and rc == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
