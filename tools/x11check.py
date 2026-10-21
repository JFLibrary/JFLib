#!/usr/bin/env python3
"""Interactive checks of the Linux display backends through WSLg (Windows host).

Starts a Linux build in WSL with jfgui_BACKEND=x11|wayland, finds its RAIL window
("<title> (<distro>)"), posts mouse / keyboard messages to it (never SendInput: the
real mouse and keyboard stay untouched), captures it and measures idle CPU time.

  python tools/x11check.py kritzel [--backend x11] [--exe build/linux/Kritzel]
      capture, Ctrl+A Ctrl+C -> image/png for another X client (X11), idle CPU
      (/proc/PID/stat over 3 s), title-bar drag (window moves), maximize / restore
      buttons, close button -> exit 0
  python tools/x11check.py keys [--backend x11] [--exe build/linux/linuxtest]
      German layout text: q Q AltGr+q=@ AltGr+e=EUR a-umlaut Shift+7=/ ... (E_TEXT)
  python tools/x11check.py clip [--exe build/linux/linuxtest]
      X11 clipboard both ways against a small Python X client (UTF-8, INCR both ways),
      X11 <-> Wayland through Weston's selection bridge
  python tools/x11check.py present [--exe build/linux/linuxtest]
      partial presents through MIT-SHM (memfd), MIT-SHM (SysV) and PutImage: the
      captured window must equal the saved framebuffer
  python tools/x11check.py dpi [--exe build/linux/linuxtest]
      Xft.dpi (root RESOURCE_MANAGER) at start and while running, jfgui_SCALE
  python tools/x11check.py conn [--exe build/linux/linuxtest]
      DISPLAY forms (:N, unix:N.S, /path:N), abstract and file sockets, Xauthority choice
      (through a recording proxy), TCP refused, no server, automatic backend choice
  python tools/x11check.py grab [--backend x11] [--exe build/linux/linuxtest]
      plat_begin_resize / plat_begin_move through the window manager
  python tools/x11check.py windows [--backend x11|wayland] [--exe build/linux/gallery]
      tool / dialog windows of the gallery ("Fenster" section) as their own windows: the
      tool window appears (own RAIL window) and shows its framebuffer; X11: a Python X
      client reads WM_TRANSIENT_FOR, _NET_WM_WINDOW_TYPE, _NET_WM_STATE, WM_HINTS (window
      group), _MOTIF_WM_HINTS, WM_NORMAL_HINTS, the position (main client origin + 24, 48)
      and a move by the window manager comes back as E_WINPOS; Wayland: the
      jfgui_WLDEBUG trace shows xdg_toplevel.set_parent; close button, modal dialog
      (X11: type DIALOG + state MODAL), "Abbrechen" closes it
  python tools/x11check.py popups [--backend x11|wayland] [--exe build/linux/gallery]
      (U3) the menu of the gallery's small tool window "Popups" (--popups-window) is an own
      window (X11 override-redirect / Wayland xdg_popup) below the button, reaching beyond
      the tool window; a click on a row arrives as the tool window's press and closes it
  python tools/x11check.py edges [--backend x11|wayland] [--exe build/linux/gallery]
      (ROADMAP 6) popups of the MAIN window (gallery --popup-edges, 600 x 360): the combo
      box's list (taller than the window) is an own window (X11 override-redirect / Wayland
      xdg_popup) reaching below the main window, a click on its last row arrives as the main
      window's press and closes it; X11 knows the work area: the calendar opens below its
      button, out of the window (Wayland keeps a popup that fits inside the window)
  python tools/x11check.py all [--bindir build/linux] [--out-dir build]
      every check on X11, kritzel + keys + grab + windows + popups + edges on Wayland

WSLg shows undecorated X11 windows with a 32 px transparent margin (Weston XWM
shadow); the margin is measured from the capture.  Exit code 0 = all checks passed.
"""
import argparse
import ctypes
import json
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
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.PostMessageW.restype = wt.BOOL

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DISTRO = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")
sys.path.insert(0, os.path.join(ROOT, "tools"))
import wslcap  # noqa: E402  (windows_with_title, capture, write_png)

WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP = 0x200, 0x201, 0x202
WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP = 0x100, 0x101, 0x104, 0x105
MK_LBUTTON = 1
FAILS = []


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  (" + detail + ")") if detail else ""))
    if not ok:
        FAILS.append(name)
    return ok


def wsl(cmd, **kw):
    return subprocess.run(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "sh", "-c", cmd],
                          capture_output=True, text=True, encoding="utf-8", errors="replace", **kw)


class App:
    """A Linux app started through WSL; stdout+stderr go to a log file."""

    def __init__(self, exe, args, backend, title, env=None, log="build/x11check.log"):
        self.title = title
        self.log = log
        envs = {"jfgui_BACKEND": backend}
        envs.update(env or {})
        envstr = " ".join(f"{k}={v}" for k, v in envs.items())
        exe = exe.replace("\\", "/")
        if not exe.startswith("/"):
            exe = "./" + exe
        self.name = os.path.basename(exe)
        self.old = {w[0] for w in wslcap.windows_with_title(title)}   # windows of earlier runs
        cmd = f"echo $$ > {log}.pid; exec env {envstr} {exe} {' '.join(args)} > {log} 2>&1"
        self.proc = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "sh", "-c", cmd])
        self.hwnd = None

    def pid(self):
        try:
            return int(open(os.path.join(ROOT, self.log + ".pid")).read().strip())
        except Exception:
            return 0

    def window(self, timeout=15):
        t = time.time() + timeout
        while time.time() < t:
            w = [x for x in wslcap.windows_with_title(self.title) if x[0] not in self.old]
            if w:
                self.hwnd = w[0][0]
                return self.hwnd
            time.sleep(0.2)
        return None

    def rect(self):
        r = wt.RECT()
        user32.GetWindowRect(self.hwnd, ctypes.byref(r))
        return r.left, r.top, r.right, r.bottom

    def output(self):
        try:
            return open(os.path.join(ROOT, self.log), encoding="utf-8", errors="replace").read()
        except Exception:
            return ""

    def wait(self, timeout=10):
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            return None

    def kill(self):
        wsl(f"pkill -f '{self.name}' || true")
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def lparam(x, y):
    return ((y & 0xFFFF) << 16) | (x & 0xFFFF)


def mouse(hwnd, msg, x, y, wparam=0):
    user32.PostMessageW(hwnd, msg, wparam, lparam(x, y))


def click(hwnd, x, y):
    mouse(hwnd, WM_MOUSEMOVE, x, y)
    time.sleep(0.15)
    mouse(hwnd, WM_LBUTTONDOWN, x, y, MK_LBUTTON)
    time.sleep(0.1)
    mouse(hwnd, WM_LBUTTONUP, x, y)
    time.sleep(0.3)


def drag(hwnd, x0, y0, x1, y1, steps=10):
    mouse(hwnd, WM_MOUSEMOVE, x0, y0)
    time.sleep(0.2)
    mouse(hwnd, WM_LBUTTONDOWN, x0, y0, MK_LBUTTON)
    time.sleep(0.3)
    for i in range(1, steps + 1):
        x = x0 + (x1 - x0) * i // steps
        y = y0 + (y1 - y0) * i // steps
        mouse(hwnd, WM_MOUSEMOVE, x, y, MK_LBUTTON)
        time.sleep(0.05)
    time.sleep(0.2)
    mouse(hwnd, WM_LBUTTONUP, x1, y1)
    time.sleep(0.5)


# scan codes (set 1) for the keys used; ext = extended key
SC = {"shift": 0x2A, "ctrl": 0x1D, "altgr": (0x38, True), "q": 0x10, "e": 0x12, "7": 0x08,
      "a": 0x1E, "adia": 0x28, "ss": 0x0C, "enter": 0x1C, "left": (0x4B, True), "ypos": 0x15, "c": 0x2E}
VK = {"shift": 0x10, "ctrl": 0x11, "altgr": 0x12, "q": 0x51, "e": 0x45, "7": 0x37, "a": 0x41,
      "adia": 0xDE, "ss": 0xDB, "enter": 0x0D, "left": 0x25, "ypos": 0x59, "c": 0x43}


def key(hwnd, name, down):
    sc = SC[name]
    ext = False
    if isinstance(sc, tuple):
        sc, ext = sc
    lp = 1 | (sc << 16) | ((1 << 24) if ext else 0)
    if not down:
        lp |= (1 << 30) | (1 << 31)
    msg = WM_KEYDOWN if down else WM_KEYUP
    if name == "altgr":
        msg = WM_SYSKEYDOWN if down else WM_SYSKEYUP
    user32.PostMessageW(hwnd, msg, VK[name], lp)
    time.sleep(0.06)


def chord(hwnd, *names):
    for n in names:
        key(hwnd, n, True)
    for n in reversed(names):
        key(hwnd, n, False)
    time.sleep(0.1)


def proc_cpu_ticks(pid):
    r = wsl(f"cat /proc/{pid}/stat")
    f = r.stdout.rsplit(")", 1)[-1].split()
    return int(f[11]) + int(f[12])          # utime + stime (fields 14, 15)


def find_child_pid(app):
    r = wsl(f"pgrep -f '{app.name}' | head -5")
    pids = [int(p) for p in r.stdout.split() if p.strip().isdigit()]
    return pids[0] if pids else 0


def wait_content(hwnd, timeout=8):
    """Capture once the window shows something (WSLg displays new RAIL windows late)."""
    t = time.time() + timeout
    while True:
        w, h, px, pos = wslcap.capture(hwnd, False)
        o = ((h // 2) * w + w // 2) * 4
        if (w and px[o:o + 3] != bytes(3)) or time.time() > t:
            return w, h, px, pos
        time.sleep(0.25)


def margin_of(png_w, png_h, px):
    """Width of the transparent (black) shadow margin at the left edge."""
    y = png_h // 2
    for x in range(min(64, png_w)):
        b, g, r = px[(y * png_w + x) * 4:(y * png_w + x) * 4 + 3]
        if r or g or b:
            return x
    return 0


# ------------------------------------------------------------------ checks

def content_size(app, default):
    """Last E_RESIZE of the app's --log-events output, else default."""
    size = default
    for line in app.output().splitlines():
        if line.startswith("event type=8 "):
            f = dict(kv.split("=") for kv in line.split()[1:] if "=" in kv)
            size = (int(f["x"]), int(f["y"]))
    return size


def lt_exe(a):
    return a.exe if a.exe and "linuxtest" in a.exe else a.bindir + "/linuxtest"


def check_kritzel(a):
    backend = a.backend
    print(f"--- Kritzel on {backend}")
    app = App(a.exe if a.exe and "Kritzel" in a.exe else a.bindir + "/Kritzel", ["--log-events"], backend, "Kritzel",
              log=f"build/x11check_kritzel_{backend}.log")
    hwnd = app.window()
    if not check(hwnd is not None, f"{backend}: Kritzel window appears"):
        app.kill()
        return
    w, h, px, pos = wait_content(hwnd)
    time.sleep(0.5)
    w, h, px, pos = wslcap.capture(hwnd, False)
    out = os.path.join(a.out_dir, f"x11check_kritzel_{backend}.png")
    wslcap.write_png(out, w, h, px)
    m = margin_of(w, h, px)          # WSLg: 32 px shadow margin around undecorated X11 windows
    if m == 0 and backend == "x11":
        # a screen capture (PrintWindow fallback) shows the desktop in the transparent
        # margin, so the pixel test can not find it: WSLg's Weston always uses 32 px
        m = 32
    cw, ch = content_size(app, (w - 2 * m, h - 2 * m))
    if cw < w:
        # the app reported its size: the margin follows from it (a screen capture shows the
        # desktop in the transparent margin, so the pixel test above can not see it)
        m = (w - cw) // 2
    print(f"window {w}x{h}, margin {m}, content {cw}x{ch} -> {out}")
    if backend == "x11":
        # Ctrl+A, Ctrl+C: Kritzel offers the canvas as image/png to other X clients
        fg = user32.GetForegroundWindow()
        user32.SetForegroundWindow(hwnd)
        time.sleep(0.8)
        chord(hwnd, "ctrl", "a")
        chord(hwnd, "ctrl", "c")
        time.sleep(1.0)
        write_pyx()
        r = wsl("timeout 10 python3 build/x11check_pyx.py get image/png")
        check(r.stdout.startswith("GOT ") and "89504e470d0a1a0a" in r.stdout,
              "x11: Kritzel Ctrl+A Ctrl+C -> image/png for other X clients", r.stdout.strip() or r.stderr.strip())
        if fg:
            user32.SetForegroundWindow(fg)
    # idle CPU
    pid = find_child_pid(app)
    if pid:
        t0 = proc_cpu_ticks(pid)
        time.sleep(3.0)
        t1 = proc_cpu_ticks(pid)
        check(t1 - t0 <= 3, f"{backend}: idle CPU", f"{t1 - t0} ticks in 3 s")
    # title bar drag: the window must move
    r0 = app.rect()
    drag(hwnd, m + 500, m + 16, m + 600, m + 90)
    time.sleep(0.8)
    r1 = app.rect()
    check((r1[0] - r0[0], r1[1] - r0[1]) != (0, 0), f"{backend}: title bar drag moves the window",
          f"{r0[:2]} -> {r1[:2]}")
    # maximize button (second from the right, 46 px wide buttons)
    click(hwnd, m + cw - 69, m + 16)
    time.sleep(1.5)
    cw2, ch2 = content_size(app, (cw, ch))
    check(cw2 > cw + 100, f"{backend}: maximize button", f"{cw}x{ch} -> {cw2}x{ch2}")
    wslcap.write_png(os.path.join(a.out_dir, f"x11check_kritzel_{backend}_max.png"),
                     *wslcap.capture(hwnd, False)[:3])
    # WSLg quirk: a maximized undecorated X11 window is drawn with the 32 px shadow
    # offset, but pointer input is mapped without it
    click(hwnd, cw2 - 69, 16)                   # restore
    time.sleep(1.5)
    cw3, ch3 = content_size(app, (cw2, ch2))
    check((cw3, ch3) == (cw, ch), f"{backend}: restore button", f"-> {cw3}x{ch3}")
    # (edge resizing: the UI core has no resize band that calls plat_begin_resize yet;
    # "x11check.py grab" tests plat_begin_resize itself)
    # close button
    time.sleep(1.0)                             # WSLg: let the restored window settle
    click(hwnd, m + cw3 - 23, m + 16)
    rc = app.wait(4)
    if rc is None and "event type=13" not in app.output():
        print("(close click lost in WSLg, clicking again)")
        click(hwnd, m + cw3 - 23, m + 16)
        rc = app.wait(8)
    check(rc == 0, f"{backend}: close button ends the app with exit code 0", f"exit {rc}")
    if rc is None:
        wslcap.write_png(os.path.join(a.out_dir, f"x11check_kritzel_{backend}_noclose.png"),
                         *wslcap.capture(hwnd, False)[:3])
        app.kill()


def check_keys(a):
    backend = a.backend
    print(f"--- keyboard on {backend}")
    app = App(lt_exe(a), ["--test", "show", "--hold", "12000"], backend,
              "jfgui Linux test", log=f"build/x11check_keys_{backend}.log")
    hwnd = app.window()
    if not check(hwnd is not None, f"{backend}: test window appears"):
        app.kill()
        return
    time.sleep(1.5)
    fg = user32.GetForegroundWindow()
    user32.SetForegroundWindow(hwnd)            # WSLg forwards keys to the active window
    time.sleep(0.8)
    seq = [("q",), ("shift", "q"), ("ctrl", "altgr", "q"), ("ctrl", "altgr", "e"), ("adia",),
           ("shift", "7"), ("ss",), ("shift", "ss"), ("ypos",), ("enter",), ("left",)]
    for c in seq:
        chord(hwnd, *c)
    r = app.rect()
    cx, cy = (r[0] + r[2]) // 2, (r[1] + r[3]) // 2
    for msg, delta in ((0x20A, 120), (0x20A, -120), (0x20E, 120), (0x20E, -120)):   # WM_MOUSE(H)WHEEL
        mouse(hwnd, WM_MOUSEMOVE, (r[2] - r[0]) // 2, (r[3] - r[1]) // 2)
        user32.PostMessageW(hwnd, msg, (delta & 0xFFFF) << 16, lparam(cx, cy))
        time.sleep(0.2)
    time.sleep(1.0)
    if fg:
        user32.SetForegroundWindow(fg)
    app.kill()
    out = app.output()
    texts, downs = [], []
    for line in out.splitlines():
        p = line.split()
        if len(p) >= 12 and p[0] == "event":
            t, data = int(p[1]), int(p[11], 16)
            if t == 7:
                texts.append(chr(data))
            elif t == 5:
                downs.append(data & 0xFFFF)
    got = "".join(texts)
    want = "qQ@€ä/ß?z"
    check(got == want, f"{backend}: German text (q Q @ EUR a-umlaut / sharp-s ? z)", repr(got))
    check(0x0D in downs and 0x25 in downs, f"{backend}: VK_RETURN and VK_LEFT key downs",
          " ".join(f"{d:X}" for d in downs))
    check(0x5A in downs, f"{backend}: key at the US-Y position gives VK 'Z' (German layout)")
    wheels = [int(l.split()[11], 16) for l in out.splitlines() if l.split()[:2] == ["event", "4"]]
    # (posted wheel messages are unreliable in WSLg: often only positive deltas arrive;
    # the X11 button -> wheel mapping is covered by linuxtest --test x11synth)
    print(f"INFO {backend}: wheel events for up / down / right / left: "
          + (" ".join(f"{w:X}" for w in wheels) or "none"))


# ---- a tiny X11 client in Python for the clipboard check (runs inside WSL)
PYX = r'''
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
            if p[0] == 0: raise SystemExit("X error %d" % p[1])
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
    def getprop(s, win, prop, delete=1):
        r = s.reply(s.req(struct.pack("<BBHIIIII", 20, delete, 6, win, prop, 0, 0, 0x100000)))
        fmt = r[1]; typ, after, n = struct.unpack("<III", r[8:20])
        return typ, r[32:32 + n * fmt // 8]
x = X()
CLIP, UTF8, TARGETS, INCR, PROP = (x.atom(n) for n in (os.environ.get("PYX_SELECTION", "CLIPBOARD"),
                                                       "UTF8_STRING", "TARGETS", "INCR", "PYX_SEL"))
mode = sys.argv[1]
if mode == "rm":
    x.change(x.root, 23, 31, 8, sys.argv[2].replace("\\t", "\t").replace("\\n", "\n").encode())
    x.reply(x.req(struct.pack("<BxH", 43, 1)))
    sys.exit(0)
if mode == "rmdel":
    x.req(struct.pack("<BxHII", 19, 3, x.root, 23))
    x.reply(x.req(struct.pack("<BxH", 43, 1)))
    sys.exit(0)
if mode == "get":
    target = x.atom(sys.argv[2]) if len(sys.argv) > 2 else UTF8
    x.req(struct.pack("<BxHIIIII", 24, 6, x.win, CLIP, target, PROP, 0))
    while True:
        e = x.event()
        if e[0] & 0x7F == 31: break
    if struct.unpack("<I", e[20:24])[0] == 0: print("REFUSED"); sys.exit(1)
    typ, data = x.getprop(x.win, PROP)
    if typ == INCR:
        data = b""
        while True:
            e = x.event()
            if e[0] & 0x7F != 28 or e[16] != 0: continue
            typ, chunk = x.getprop(x.win, PROP)
            if not chunk: break
            data += chunk
    if target != UTF8:
        print("GOT %d %s" % (len(data), data[:8].hex()))
    else:
        print("GOT %d %s" % (len(data), data[:60].decode("utf-8", "replace")))
    sys.exit(0)
text = sys.argv[2].encode() if mode == "set" else (b"0123456789abcdef" * int(sys.argv[2]) + b"END")
limit = 16384 if mode == "big" else 100000                  # INCR above this, chunks of limit / 2
x.req(struct.pack("<BxHIII", 22, 4, x.win, CLIP, 0))
print("OWNER", flush=True)
end = time.time() + float(sys.argv[3] if len(sys.argv) > 3 else 20)
incr = {}
while time.time() < end:
    try: e = x.event(1)
    except socket.timeout: continue
    t = e[0] & 0x7F
    if t == 30:
        tm, owner, req, sel, target, prop = struct.unpack("<IIIIII", e[4:28])
        if target == TARGETS:
            x.change(req, prop, 4, 32, struct.pack("<II", TARGETS, UTF8))
        elif target == UTF8 and len(text) <= limit:
            x.change(req, prop, UTF8, 8, text)
        elif target == UTF8:
            x.req(struct.pack("<BxHIII", 2, 4, req, 0x800, 0x400000))
            x.change(req, prop, INCR, 32, struct.pack("<I", len(text)))
            incr[(req, prop)] = 0
        else:
            prop = 0
        x.req(struct.pack("<BBHII", 25, 0, 11, req, 0) + struct.pack("<BxHIIIII8x", 31, 0, tm, req, sel, target, prop))
    elif t == 28 and e[16] == 1:
        win, prop = struct.unpack("<II", e[4:12])
        if (win, prop) in incr:
            off = incr[(win, prop)]
            chunk = text[off:off + limit // 2]
            x.change(win, prop, UTF8, 8, chunk)
            incr[(win, prop)] = off + len(chunk)
            if not chunk: del incr[(win, prop)]; print("INCR DONE", flush=True)
    elif t == 29:
        print("LOST", flush=True); break
'''


def write_pyx():
    with open(os.path.join(ROOT, "build", "x11check_pyx.py"), "w", encoding="utf-8", newline="\n") as f:
        f.write(PYX)


def check_dpi(a):
    """Xft.dpi in RESOURCE_MANAGER at start and changed while running; jfgui_SCALE."""
    print("--- dpi on x11")
    write_pyx()
    exe = lt_exe(a)
    wsl("python3 build/x11check_pyx.py rm 'Xft.dpi:\\t144\\nXft.antialias:\\t1\\n'")
    try:
        app = App(exe, ["--test", "show", "--hold", "7000"], "x11", "jfgui Linux test",
                  log="build/x11check_dpi1.log")
        app.window()
        time.sleep(2.5)
        wsl("python3 build/x11check_pyx.py rm 'Xft.dpi:\\t192\\n'")
        app.wait(15)
        out = app.output()
        check("window 960 x 600 px, dpi 144" in out, "x11: Xft.dpi 144 at start -> 960 x 600 px",
              next((l for l in out.splitlines() if l.startswith("window")), "?"))
        check("event 9 btn 0 mods 0 x 0 y 0 data c0" in out, "x11: Xft.dpi changed to 192 -> E_DPI 192")
        check("event 8 btn 0 mods 0 x 1280 y 800" in out, "x11: ... and the window keeps its DIP size (1280 x 800)")
    finally:
        wsl("python3 build/x11check_pyx.py rmdel")
    app = App(exe, ["--test", "show", "--hold", "1500"], "x11", "jfgui Linux test", env={"jfgui_SCALE": "2"},
              log="build/x11check_dpi2.log")
    app.wait(15)
    out = app.output()
    check("window 1280 x 800 px, dpi 192" in out, "x11: jfgui_SCALE=2 -> dpi 192",
          next((l for l in out.splitlines() if l.startswith("window")), "?"))
    app = App(exe, ["--test", "show", "--hold", "1500"], "x11", "jfgui Linux test", log="build/x11check_dpi3.log")
    app.wait(15)
    out = app.output()
    check("window 640 x 400 px, dpi 96" in out, "x11: no Xft.dpi -> 96",
          next((l for l in out.splitlines() if l.startswith("window")), "?"))


def check_clip(a):
    print("--- clipboard on x11")
    write_pyx()
    exe = lt_exe(a)
    # 1: another X client owns UTF-8 text -> our clip_get reads it
    owner = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "python3", "build/x11check_pyx.py",
                              "set", "Grüße aus X11 €", "8"], stdout=subprocess.PIPE, text=True)
    time.sleep(1.0)
    app = App(exe, ["--test", "clip", "--hold", "2500"], "x11", "jfgui Linux test", log="build/x11check_clip1.log")
    app.window()
    rc = app.wait(20)
    out = app.output()
    check("clipboard before: [Grüße aus X11 €]" in out, "x11: paste UTF-8 text from another X client",
          next((l for l in out.splitlines() if "clipboard before" in l), "?"))
    check(rc == 0, "x11: linuxtest --test clip passes", f"exit {rc}")
    # 2: while the app holds its text, the Python client reads it (UTF8_STRING)
    app = App(exe, ["--test", "clip", "--hold", "6000"], "x11", "jfgui Linux test", log="build/x11check_clip2.log")
    app.window()
    time.sleep(3.0)
    r = wsl("timeout 10 python3 build/x11check_pyx.py get")
    check("GOT" in r.stdout and "jfgui Wayland clipboard: Grüße € ✓" in r.stdout,
          "x11: another X client pastes our text", r.stdout.strip() or r.stderr.strip())
    app.wait(15)
    owner.kill()
    # 3: X11 <-> Wayland (Weston) selection bridge -- the path WSLg forwards to Windows
    x = App(exe, ["--test", "clip", "--text", "'X11 → Wayland Größe'", "--hold", "7000"], "x11",
            "jfgui Linux test", log="build/x11check_clip5.log")
    x.window()
    time.sleep(3.0)
    w = App(exe, ["--test", "clip", "--hold", "0"], "wayland", "jfgui Linux test",
            log="build/x11check_clip6.log")
    w.wait(20)
    x.wait(20)
    out = w.output()
    check("clipboard before: [X11 → Wayland Größe]" in out, "x11 -> wayland: a Wayland client pastes our X11 text",
          next((l for l in out.splitlines() if "clipboard before" in l), "?"))
    w = App(exe, ["--test", "clip", "--text", "'Wayland → X11 ½'", "--hold", "7000"], "wayland",
            "jfgui Linux test", log="build/x11check_clip7.log")
    w.window()
    time.sleep(3.0)
    x = App(exe, ["--test", "clip", "--hold", "0"], "x11", "jfgui Linux test", log="build/x11check_clip8.log")
    x.wait(20)
    w.wait(20)
    out = x.output()
    check("clipboard before: [Wayland → X11 ½]" in out, "wayland -> x11: our X11 client pastes Wayland text",
          next((l for l in out.splitlines() if "clipboard before" in l), "?"))
    # 4: INCR into clip_get (40 KB in 8 KB chunks).  On a private selection: WSLg's Weston
    # 9.0 mirrors CLIPBOARD and crashes (restarting Xwayland) after reading any INCR selection
    sel = {"jfgui_X11SELECTION": "jfgui_TEST_SELECTION"}
    owner = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "env", "PYX_SELECTION=jfgui_TEST_SELECTION",
                              "python3", "build/x11check_pyx.py", "big", "2500", "10"], stdout=subprocess.PIPE, text=True)
    time.sleep(1.0)
    app = App(exe, ["--test", "clip", "--hold", "1000"], "x11", "jfgui Linux test", env=sel,
              log="build/x11check_clip3.log")
    app.window()
    app.wait(20)
    out = app.output()
    owner.kill()
    line = next((l for l in out.splitlines() if "clipboard before" in l), "")
    big = b"0123456789abcdef" * 2500 + b"END"
    h = 0xCBF29CE484222325
    for byte in big:
        h = ((h ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    check(line.endswith("(40003 bytes)") and f"clipboard hash {h:x}" in out,
          "x11: paste 40 KB through INCR, 8 KB chunks (length + FNV hash)", line[-30:])
    # 5: we offer 40 KB through INCR (jfgui_X11INCR=4096: 2 KB chunks)
    app = App(exe, ["--test", "clip", "--big", "2500", "--hold", "6000"], "x11", "jfgui Linux test",
              env=dict(sel, jfgui_X11INCR="4096"), log="build/x11check_clip4.log")
    app.window()
    time.sleep(3.0)
    r = wsl("timeout 10 env PYX_SELECTION=jfgui_TEST_SELECTION python3 build/x11check_pyx.py get")
    check(r.stdout.startswith("GOT 40003 0123456789abcdef"), "x11: another X client pastes 40 KB from us (INCR)",
          r.stdout.strip()[:40] or r.stderr.strip())
    app.wait(15)
    # 6: 640 KB without INCR: one ChangeProperty through BIG-REQUESTS
    app = App(exe, ["--test", "clip", "--big", "40000", "--hold", "6000"], "x11", "jfgui Linux test",
              env=sel, log="build/x11check_clip5.log")
    app.window()
    time.sleep(3.0)
    r = wsl("timeout 10 env PYX_SELECTION=jfgui_TEST_SELECTION python3 build/x11check_pyx.py get")
    check(r.stdout.startswith("GOT 640003 0123456789abcdef"),
          "x11: another X client pastes 640 KB from us (one BIG-REQUESTS property)",
          r.stdout.strip()[:40] or r.stderr.strip())
    app.wait(15)


def find_color(w, h, px, rgb):
    """First pixel (x, y) of an exact colour in a BGRA capture, row by row."""
    r, g, b = rgb
    for y in range(h):
        row = px[y * w * 4:(y + 1) * w * 4]
        for x in range(w):
            if row[x * 4] == b and row[x * 4 + 1] == g and row[x * 4 + 2] == r:
                return x, y
    return None


def check_grab(a):
    """linuxtest --test grab: a press in the right half calls plat_begin_resize(right |
    bottom), in the left half plat_begin_move; the window manager does the rest."""
    backend = a.backend
    print(f"--- window manager move / resize on {backend}")
    app = App(lt_exe(a), ["--test", "grab", "--hold", "12000"], backend,
              "jfgui Linux test", log=f"build/x11check_grab_{backend}.log")
    hwnd = app.window()
    if not check(hwnd is not None, f"{backend}: test window appears"):
        app.kill()
        return
    t = time.time() + 6
    org = None
    while time.time() < t and org is None:          # the blue bar is at content (20, 20)
        w, h, px, _ = wslcap.capture(hwnd, False)
        org = find_color(w, h, px, (0x00, 0x78, 0xD4))
        time.sleep(0.25)
    if not check(org is not None, f"{backend}: test pattern visible"):
        app.kill()
        return
    ox, oy = org[0] - 20, org[1] - 20
    drag(hwnd, ox + 500, oy + 300, ox + 620, oy + 380, steps=12)
    time.sleep(1.0)
    out = app.output()
    sizes = [l for l in out.splitlines() if l.strip().startswith("event 8 ")]
    check("begin_resize" in out and sizes, f"{backend}: begin_resize -> the window grows",
          sizes[-1].strip() if sizes else "no E_RESIZE")
    r0 = app.rect()
    drag(hwnd, ox + 100, oy + 300, ox + 180, oy + 360, steps=12)
    time.sleep(1.0)
    r1 = app.rect()
    out = app.output()
    check("begin_move" in out and r1[:2] != r0[:2], f"{backend}: begin_move -> the window moves",
          f"{r0[:2]} -> {r1[:2]}")
    ups = [l for l in out.splitlines() if l.strip().startswith("event 3 ")]
    check(len(ups) >= 2, f"{backend}: E_UP after each grab (buttons released for the app)", f"{len(ups)} E_UP")
    app.kill()


# ---- a proxy X "server" (runs inside WSL): records the client's authentication and
# forwards the connection to the real server without it (drops passed fds, so
# ShmAttachFd fails and the client has to fall back)
PROXY = r"""
import os, select, socket, struct, sys
addr, log, timeout = sys.argv[1], sys.argv[2], float(sys.argv[3])
def pad4(n): return (n + 3) & ~3
def recvn(c, n):
    b = b""
    while len(b) < n:
        x = c.recv(n - len(b))
        if not x: raise SystemExit(0)
        b += x
    return b
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
if addr.startswith("@"):
    srv.bind("\0" + addr[1:])
else:
    try: os.unlink(addr)
    except FileNotFoundError: pass
    srv.bind(addr)
srv.listen(4)
print("READY", flush=True)
srv.settimeout(timeout)
try:
    c, _ = srv.accept()
except socket.timeout:
    raise SystemExit(0)
hdr = recvn(c, 12)
n, d = struct.unpack("<HH", hdr[6:10])
rest = recvn(c, pad4(n) + pad4(d))
open(log, "w").write("AUTH %s %s\n" % (rest[:n].decode(), rest[pad4(n):pad4(n) + d].hex()))
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("/tmp/.X11-unix/X0")
s.sendall(hdr[:6] + b"\0\0\0\0" + hdr[10:12])
while True:
    r, _, _ = select.select([c, s], [], [])
    for a in r:
        b = a.recv(65536)
        if not b:
            if not addr.startswith("@"): os.unlink(addr)
            raise SystemExit(0)
        (s if a is c else c).sendall(b)
"""


def check_conn(a):
    """DISPLAY forms, abstract / file sockets, Xauthority choice, TCP refusal, no server,
    automatic backend choice."""
    print("--- connection on x11")
    exe = (lt_exe(a)).replace("\\", "/")
    with open(os.path.join(ROOT, "build", "x11check_proxy.py"), "w", encoding="utf-8", newline="\n") as f:
        f.write(PROXY)
    host = wsl("uname -n").stdout.strip().encode()

    def rec(family, addr, num, data):
        f = lambda b: struct.pack(">H", len(b)) + b
        return struct.pack(">H", family) + f(addr) + f(num) + f(b"MIT-MAGIC-COOKIE-1") + f(data)
    ck_other, ck_host, ck_wild = bytes(range(16)), bytes(range(16, 32)), bytes(range(32, 48))
    with open(os.path.join(ROOT, "build", "x11check.xauth"), "wb") as f:
        f.write(rec(256, b"otherhost", b"57", ck_other) + rec(0, b"\x7f\0\0\1", b"57", ck_other)
                + rec(256, host, b"57", ck_host) + rec(65535, b"", b"", ck_wild))
    cases = [("@/tmp/.X11-unix/X57", ":57", ck_host, "abstract socket, DISPLAY=:57, cookie of our host"),
             ("@/tmp/.X11-unix/X58", "unix:58.0", ck_wild, "DISPLAY=unix:58.0 (abstract socket), wildcard cookie"),
             ("/tmp/x11check.sock", "/tmp/x11check.sock:0", ck_wild, "socket path DISPLAY=/path:0")]
    for addr, disp, cookie, name in cases:
        log = f"build/x11check_proxy.log"
        try:
            os.remove(os.path.join(ROOT, log))
        except OSError:
            pass
        proxy = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", ROOT, "-e", "python3", "build/x11check_proxy.py",
                                  addr, log, "15"], stdout=subprocess.PIPE, text=True)
        proxy.stdout.readline()
        r = wsl(f"env jfgui_BACKEND=x11 jfgui_X11DEBUG=1 DISPLAY='{disp}' XAUTHORITY=build/x11check.xauth "
                f"./{exe} --test show --hold 800 2>&1")
        try:
            got = open(os.path.join(ROOT, log)).read().strip()
        except OSError:
            got = "no connection"
        check(r.returncode == 0 and got == "AUTH MIT-MAGIC-COOKIE-1 " + cookie.hex(), "x11: " + name,
              f"exit {r.returncode}, {got[:60]}")
        if disp == ":57":
            # the proxy drops passed fds: ShmAttachFd must fail and SysV take over
            check("ShmAttachFd failed" in r.stdout and "MIT-SHM request" not in r.stdout,
                  "x11: ShmAttachFd refused -> SysV shared memory", " | ".join(
                      l for l in r.stdout.splitlines() if "ShmAttach" in l or "error" in l or "PutImage" in l)[:120])
        proxy.kill()
    r = wsl(f"env jfgui_BACKEND=x11 DISPLAY=localhost:10 ./{exe} --test show --hold 300 2>&1")
    check(r.returncode == 1 and "TCP display" in r.stdout, "x11: DISPLAY=localhost:10 refused with a message",
          r.stdout.strip().splitlines()[0] if r.stdout.strip() else "")
    r = wsl(f"env jfgui_BACKEND=x11 DISPLAY=:77 ./{exe} --test show --hold 300 2>&1")
    lines = r.stdout.strip().splitlines()
    check(r.returncode == 1 and len(lines) == 1 and lines[0].startswith("error: no display"),
          "x11: no X server -> only the dispatcher's error, exit 1", " | ".join(lines)[:100])
    r = wsl(f"env -u WAYLAND_DISPLAY jfgui_X11DEBUG=1 DISPLAY=:0 ./{exe} --test show --hold 300 2>&1")
    check(r.returncode == 0 and "x11: connected" in r.stdout, "auto: only DISPLAY set -> X11")
    r = wsl(f"env WAYLAND_DISPLAY=no-such-socket jfgui_X11DEBUG=1 DISPLAY=:0 ./{exe} --test show --hold 300 2>&1")
    check(r.returncode == 0 and "x11: connected" in r.stdout, "auto: Wayland unreachable -> falls back to X11")
    r = wsl(f"env jfgui_X11DEBUG=1 ./{exe} --test show --hold 300 2>&1")
    check(r.returncode == 0 and "x11:" not in r.stdout, "auto: WAYLAND_DISPLAY set -> Wayland (no X11 connection)")


def read_png(path):
    """8-bit RGB / RGBA non-interlaced PNG -> (w, h, bytes RGB)."""
    data = open(path, "rb").read()
    pos, idat, w = 8, b"", 0
    while pos < len(data):
        n = struct.unpack(">I", data[pos:pos + 4])[0]
        t = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + n]
        if t == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif t == b"IDAT":
            idat += body
        pos += 12 + n
    bpp = 4 if ctype == 6 else 3
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = bytearray(h * stride)
    prev = bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out[y * stride:(y + 1) * stride] = line
        prev = line
    rgb = bytearray(w * h * 3)
    for i in range(w * h):
        rgb[i * 3:i * 3 + 3] = out[i * bpp:i * bpp + 3]
    return w, h, bytes(rgb)


def compare_capture(cw, ch, cpx, fw, fh, frgb):
    """Find the framebuffer inside the capture (BGRA); fraction of equal pixels (+-8)."""
    def cap(x, y):
        o = (y * cw + x) * 4
        return cpx[o + 2], cpx[o + 1], cpx[o]
    grid = [(x, y) for y in range(3, fh, 37) for x in range(5, fw, 41)]
    samples = grid[::max(1, len(grid) // 40)]
    samples += [(x, e) for e in (0, 1, fh - 2, fh - 1) for x in range(7, fw, fw // 7)]
    samples += [(e, y) for e in (0, 1, fw - 2, fw - 1) for y in range(7, fh, fh // 7)]
    best = None
    for oy in range(0, ch - fh + 1):
        for ox in range(0, cw - fw + 1):
            ok = True
            for (x, y) in samples:
                r, g, b = cap(ox + x, oy + y)
                o = (y * fw + x) * 3
                if abs(r - frgb[o]) > 8 or abs(g - frgb[o + 1]) > 8 or abs(b - frgb[o + 2]) > 8:
                    ok = False
                    break
            if ok:
                best = (ox, oy)
                break
        if best:
            break
    if not best:
        return 0.0, None
    ox, oy = best
    good = 0
    for y in range(fh):
        for x in range(0, fw, 2):
            r, g, b = cap(ox + x, oy + y)
            o = (y * fw + x) * 3
            if abs(r - frgb[o]) <= 8 and abs(g - frgb[o + 1]) <= 8 and abs(b - frgb[o + 2]) <= 8:
                good += 1
    return good / (fh * ((fw + 1) // 2)), best


def check_present(a):
    """linuxtest --test anim with each X11 present path; the window must show the
    framebuffer that was saved after the last frame."""
    for mode, want in (("fd", 1), ("sysv", 2), ("off", -1)):
        print(f"--- present path {mode} on x11")
        fb = f"build/x11check_fb_{mode}.png"
        try:
            os.remove(os.path.join(ROOT, fb))           # never compare with an old run's file
        except OSError:
            pass
        app = App(lt_exe(a), ["--test", "anim", "--hold", "1500", "--save", fb,
                                                     "--after", "6000"], "x11",
                  "jfgui Linux test", env={"jfgui_X11SHM": mode, "jfgui_X11DEBUG": "1"},
                  log=f"build/x11check_present_{mode}.log")
        hwnd = app.window()
        if not check(hwnd is not None, f"x11 {mode}: window appears"):
            app.kill()
            continue
        t = time.time() + 10
        while time.time() < t and not os.path.exists(os.path.join(ROOT, fb)):
            time.sleep(0.1)
        time.sleep(0.5)                        # (the PNG is complete)
        fw, fh, frgb = read_png(os.path.join(ROOT, fb))
        t = time.time() + 6                    # WSLg shows RAIL windows late: poll
        while True:
            cw, ch, cpx, _ = wslcap.capture(hwnd, False)
            frac, off = compare_capture(cw, ch, cpx, fw, fh, frgb) if cw else (0.0, None)
            if frac > 0.999 or time.time() > t:
                break
            time.sleep(0.4)
        wslcap.write_png(os.path.join(a.out_dir, f"x11check_present_{mode}.png"), cw, ch, cpx)
        rc = app.wait(15)
        out = app.output()
        check(f"present mode {want} " in out, f"x11 {mode}: present mode {want} used",
              next((l for l in out.splitlines() if "present mode" in l), "?"))
        frames = next((l for l in out.splitlines() if l.startswith("anim:")), "anim: ?")
        check(frac > 0.999, f"x11 {mode}: window shows the framebuffer after partial presents",
              f"{frac * 100:.2f} % equal at {off}, {frames}")
        check(rc == 0, f"x11 {mode}: exit code 0", f"exit {rc}")


# ---- windows: tool / dialog windows of the gallery as real X11 / Wayland windows

# a Python X client (inside WSL): the properties of our windows by title (JSON)
XPROPS = r'''
import json, os, socket, struct, sys
def pad(n): return (4 - n % 4) % 4
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("/tmp/.X11-unix/X" + os.environ.get("DISPLAY", ":0").split(":")[1].split(".")[0])
s.sendall(struct.pack("<BxHHHHxx", 0x6C, 11, 0, 0, 0))
def read(n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c: raise SystemExit("eof")
        b += c
    return b
h = read(8); body = read(struct.unpack("<H", h[6:8])[0] * 4)
vl = struct.unpack("<H", body[16:18])[0]; nf = body[21]
root = struct.unpack("<I", body[32 + vl + pad(vl) + 8 * nf:][:4])[0]
seq = [0]
def req(d):
    s.sendall(d); seq[0] += 1; return seq[0]
def reply(n):
    while True:
        p = read(32)
        if p[0] == 1: p += read(struct.unpack("<I", p[4:8])[0] * 4)
        if p[0] == 0:
            if struct.unpack("<H", p[2:4])[0] == n & 0xFFFF: return None
            continue
        if p[0] == 1 and struct.unpack("<H", p[2:4])[0] == n & 0xFFFF: return p
def atom(name):
    b = name.encode()
    return struct.unpack("<I", reply(req(struct.pack("<BBHHxx", 16, 0, 2 + (len(b) + pad(len(b))) // 4, len(b)) + b + b"\0" * pad(len(b))))[8:12])[0]
def aname(a):
    r = reply(req(struct.pack("<BxHI", 17, 2, a)))
    if not r: return "?"
    n = struct.unpack("<H", r[8:10])[0]; return r[32:32 + n].decode()
def prop(w, p, typ=0):
    r = reply(req(struct.pack("<BBHIIIII", 20, 0, 6, w, p, typ, 0, 1000)))
    if not r: return None, b""
    fmt = r[1]; t, after, n = struct.unpack("<III", r[8:20])
    return t, r[32:32 + n * fmt // 8]
def children(w):
    r = reply(req(struct.pack("<BxHI", 15, 2, w)))
    if not r: return []
    n = struct.unpack("<H", r[16:18])[0]
    return list(struct.unpack("<%dI" % n, r[32:32 + 4 * n]))
def u32s(b): return list(struct.unpack("<%dI" % (len(b) // 4), b[:len(b) // 4 * 4]))
WM_NAME, NET_NAME = 39, atom("_NET_WM_NAME")
out = {}
chk = u32s(prop(root, atom("_NET_SUPPORTING_WM_CHECK"))[1])
out["_wm"] = prop(chk[0], NET_NAME)[1].decode("utf-8", "replace") if chk else ""
move = [a for a in sys.argv[1:] if a.startswith("move=")]
wmmove = [a for a in sys.argv[1:] if a.startswith("wmmove=")]
img = [a for a in sys.argv[1:] if a.startswith("getimage=")]
names = [a for a in sys.argv[1:] if not a.startswith(("move=", "wmmove=", "getimage="))]
for f in children(root):
    for w in [f] + children(f):
        t, name = prop(w, NET_NAME)
        if not name: t, name = prop(w, WM_NAME)
        name = name.decode("utf-8", "replace")
        if name in names and name not in out:
            r = reply(req(struct.pack("<BxHIIhh", 40, 4, w, root, 0, 0)))
            rx, ry = struct.unpack("<hh", r[12:16]) if r else (0, 0)
            g = reply(req(struct.pack("<BxHI", 14, 2, w)))
            gw, gh = struct.unpack("<HH", g[16:20]) if g else (0, 0)
            d = {"id": w, "root": [rx, ry], "size": [gw, gh]}
            d["transient_for"] = u32s(prop(w, 68)[1])
            d["type"] = [aname(a) for a in u32s(prop(w, atom("_NET_WM_WINDOW_TYPE"))[1])]
            d["state"] = [aname(a) for a in u32s(prop(w, atom("_NET_WM_STATE"))[1])]
            d["hints"] = u32s(prop(w, 35)[1])
            d["normal_hints"] = u32s(prop(w, 40)[1])
            d["motif"] = u32s(prop(w, atom("_MOTIF_WM_HINTS"))[1])
            out[name] = d
            if img:                             # GetImage (ZPixmap, BGRX) of the whole window
                r = reply(req(struct.pack("<BBHIhhHHI", 73, 2, 5, w, 0, 0, gw, gh, 0xFFFFFFFF)))
                if r:
                    with open(img[0][9:], "wb") as fo:
                        fo.write(r[32:32 + gw * gh * 4])
                    d["image"] = [gw, gh]
            if wmmove:                          # ask the window manager to move it (EWMH
                dx, dy = (int(v) for v in wmmove[0][7:].split(","))   # _NET_MOVERESIZE_WINDOW)
                ev = struct.pack("<BBHII", 33, 32, 0, w, atom("_NET_MOVERESIZE_WINDOW")) +                     struct.pack("<5I", (1 << 8) | (1 << 9) | (2 << 12), (rx + dx) & 0xFFFFFFFF, (ry + dy) & 0xFFFFFFFF, 0, 0)
                req(struct.pack("<BBHII", 25, 0, 11, root, 0x180000) + ev)
                reply(req(struct.pack("<BxH", 43, 1)))
            if move:                            # ConfigureWindow like a window manager move
                dx, dy = (int(v) for v in move[0][5:].split(","))
                req(struct.pack("<BxHIHxxii", 12, 5, w, 3, rx + dx, ry + dy))
                reply(req(struct.pack("<BxH", 43, 1)))
print(json.dumps(out))
'''


def xprops(*titles):
    with open(os.path.join(ROOT, "build", "x11check_xprops.py"), "w", encoding="utf-8", newline="\n") as f:
        f.write(XPROPS)
    r = wsl("timeout 10 python3 build/x11check_xprops.py " + " ".join(f"'{t}'" for t in titles))
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        print("xprops:", r.stdout.strip()[-300:], r.stderr.strip()[-300:])
        return {}


def content_check(backend, title, cap, ref_png, what):
    """the RAIL capture shows the window's framebuffer (reference: headless shot); X11:
    when WSLg shows the RAIL window black (seen now and then with a new window, while the
    X window has the right pixels), the X window's own pixels (GetImage) decide"""
    w, h, px = cap
    fw, fh, frgb = read_png(os.path.join(ROOT, ref_png))
    frac, off = compare_capture(w, h, px, fw, fh, frgb)
    detail = f"{frac * 100:.2f} % equal at {off}"
    if frac < 0.97 and backend == "x11":
        path = os.path.join(ROOT, "build", "x11check_ximage.raw")
        try:
            os.remove(path)
        except OSError:
            pass
        d = xprops(title, "getimage=build/x11check_ximage.raw").get(title, {})
        size = d.get("image")
        if size and os.path.exists(path):
            xf, xo = compare_capture(size[0], size[1], open(path, "rb").read(), fw, fh, frgb)
            print(f"INFO x11: the RAIL capture of {title!r} does not show it ({detail}); "
                  f"X GetImage of the window: {xf * 100:.2f} % equal")
            if xf >= 0.97:
                frac, detail = xf, f"X GetImage {xf * 100:.2f} % equal; WSLg showed the RAIL window {detail}"
                off = None
    check(frac >= 0.97, f"{backend}: {what} shows its framebuffer", detail)
    return off


def headless_linux(exe, body, name="x11check_windows.uis", env=""):
    """run a script with the Linux build (headless) -> JSON dump records, exit code
    (env: e.g. "jfgui_CSD=1" for the library title bar the Wayland window gets)"""
    path = os.path.join(ROOT, "build", name)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    exe = exe.replace("\\", "/")
    r = wsl(f"{'env ' + env + ' ' if env else ''}./{exe} --anim off --script build/{name}")
    recs = []
    for line in r.stdout.splitlines():
        if line.startswith("{"):
            try:
                recs.append(json.loads(line))
            except ValueError:
                pass
    return recs, r.returncode


def first(recs, pred, last=False):
    hits = [r for r in recs if pred(r)]
    return (hits[-1] if last else hits[0]) if hits else None


def rail_by_title(title, old=(), timeout=8):
    t = time.time() + timeout
    while time.time() < t:
        w = [x[0] for x in wslcap.windows_with_title(title) if x[0] not in old]
        if w:
            return w[0]
        time.sleep(0.2)
    return None


def check_windows(a):
    """gallery "Fenster" section on X11 / Wayland: the tool window "Werkzeuge" and the modal
    dialog "Bild bearbeiten" become their own windows (WSLg RAIL windows)"""
    backend = a.backend
    print(f"--- tool / dialog windows on {backend}")
    exe = a.exe if a.exe and "gallery" in a.exe else a.bindir + "/gallery"
    notches = 20                                # the page scroll saturates after 13
    wheel = "".join("wheel 960,400 -120\n" for _ in range(notches))     # (no nested list there)
    tool_png = "build/x11check_ref_tool.png"
    dlg_png = "build/x11check_ref_dialog.png"
    recs, rc = headless_linux(exe, "size 1000 800\ntheme light\n" + wheel + "dump\n"
                              "click \"Werkzeugfenster\"\nexpect window 7600 open\n"
                              f"shot {tool_png} window 7600\n"
                              "click \"Dialogfenster…\"\nexpect window 7620 open\n"
                              f"shot {dlg_png} window 7620\ndump\n",
                              env="jfgui_CSD=1" if backend == "wayland" else "")
    b_tool = first(recs, lambda r: r.get("t") == "Werkzeugfenster")
    b_dlg = first(recs, lambda r: r.get("t") == "Dialogfenster…")
    w_tool = first(recs, lambda r: r.get("window") == 7600, True)
    cancel = first(recs, lambda r: r.get("t") == "Abbrechen" and r.get("w") == 7620, True)
    if not check(rc == 0 and all([b_tool, b_dlg, w_tool, cancel]), f"{backend}: headless reference run"):
        return
    env = {"jfgui_WLDEBUG": "1"} if backend == "wayland" else {}
    old_tool = {w[0] for w in wslcap.windows_with_title("Werkzeuge")}
    old_dlg = {w[0] for w in wslcap.windows_with_title("Bild bearbeiten")}
    app = App(exe, ["--size", "1000x800", "--anim", "off", "--theme", "light", "--log-events"], backend,
              "jfgui Galerie", env=env, log=f"build/x11check_windows_{backend}.log")
    hwnd = app.window()
    if not check(hwnd is not None, f"{backend}: main window appears"):
        app.kill()
        return
    try:
        run_windows(a, backend, app, hwnd, notches, b_tool, b_dlg, w_tool, cancel, tool_png, dlg_png,
                    old_tool, old_dlg)
    finally:
        app.kill()


def content_origin(app, hwnd, backend, timeout=6):
    """where the app's client area is inside the RAIL window"""
    vx, vy = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    t = time.time() + timeout
    while time.time() < t:
        if backend == "x11":
            pos = [l for l in app.output().splitlines() if l.startswith("event type=16 ") and l.endswith("win=0")]
            if pos:
                f = dict(kv.split("=") for kv in pos[-1].split()[1:])
                r = app.rect()
                return int(f["x"]) + vx - r[0], int(f["y"]) + vy - r[1]
        else:
            return 0, 0                         # Wayland: the RAIL window is the surface
        time.sleep(0.2)
    return None


def cursor_inside(hwnd):
    pt = wt.POINT()
    user32.GetCursorPos(ctypes.byref(pt))
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return r.left <= pt.x < r.right and r.top <= pt.y < r.bottom, pt, r


def avoid_cursor(hwnd, xtitle, backend):
    """WSLg sends the real pointer position with RAIL input: a window that popped up under
    the user's mouse gets its clicks at the wrong place.  X11: move it away (as a window
    manager would); Wayland can not move windows."""
    inside, pt, r = cursor_inside(hwnd)
    if not inside:
        return True
    if backend != "x11":
        print(f"WARN {backend}: the mouse pointer is over the window {xtitle!r}: clicks may land elsewhere")
        return False
    vl, vw = user32.GetSystemMetrics(76), user32.GetSystemMetrics(78)
    w = r.right - r.left
    new_left = pt.x + 60 if pt.x + 60 + w <= vl + vw else pt.x - 60 - w
    xprops(xtitle, f"move={new_left - r.left},0")
    time.sleep(1.5)
    return not cursor_inside(hwnd)[0]


def rail_click(hwnd, x, y, tries_ms=10000):
    """a click posted back to back, when the user's pointer is not over the window (WSLg
    sends the real pointer position with RAIL input, so a pointer moving over the window
    moves the release elsewhere)"""
    t = time.time() + tries_ms / 1000
    while cursor_inside(hwnd)[0] and time.time() < t:
        time.sleep(0.2)
    mouse(hwnd, WM_MOUSEMOVE, x, y)
    mouse(hwnd, WM_LBUTTONDOWN, x, y, MK_LBUTTON)
    mouse(hwnd, WM_LBUTTONUP, x, y)
    time.sleep(0.5)


def run_windows(a, backend, app, hwnd, notches, b_tool, b_dlg, w_tool, cancel, tool_png, dlg_png,
                old_tool, old_dlg):
    time.sleep(1.5)
    avoid_cursor(hwnd, "jfgui Galerie", backend)
    org = content_origin(app, hwnd, backend)
    if not check(org is not None, f"{backend}: position of the main window's client area", str(org)):
        return
    ox, oy = org
    r = app.rect()
    # scroll to the "Fenster" section: single notches until enough arrived at the right
    # place (WSLg reports the real pointer position with wheel events: wait while the
    # user's pointer is over the window)
    def wheels():
        return sum(1 for l in app.output().splitlines()
                   if l.startswith("event type=4 ") and " x=960 y=400 " in l)
    for i in range(notches + 40):
        if wheels() >= notches:
            break
        t = time.time() + 20
        while cursor_inside(hwnd)[0] and time.time() < t:
            time.sleep(0.2)
        mouse(hwnd, WM_MOUSEMOVE, ox + 960, oy + 400)
        user32.PostMessageW(hwnd, 0x20A, (-120 & 0xFFFF) << 16, lparam(r[0] + ox + 960, r[1] + oy + 400))
        time.sleep(0.15)
    time.sleep(0.5)
    n = wheels()
    check(n >= 13, f"{backend}: wheel notches arrived (13 scroll the page to the section)", str(n))
    bx, by = b_tool["r"][0] + b_tool["r"][2] // 2, b_tool["r"][1] + b_tool["r"][3] // 2
    for _ in range(3):
        rail_click(hwnd, ox + bx, oy + by)
        tool = rail_by_title("Werkzeuge", old_tool, 5)
        if tool:
            break
    if not check(tool is not None, f"{backend}: tool window \"Werkzeuge\" appears as its own window"):
        wslcap.write_png(os.path.join(a.out_dir, f"x11check_windows_{backend}_main.png"), *wslcap.capture(hwnd, False)[:3])
        return
    time.sleep(1.5)                             # WSLg shows new RAIL windows late
    main_x = xprops("jfgui Galerie", "Werkzeuge") if backend == "x11" else {}
    avoid_cursor(tool, "Werkzeuge", backend)
    w, h, px, _ = wait_content(tool)
    wslcap.write_png(os.path.join(a.out_dir, f"x11check_windows_{backend}_tool.png"), w, h, px)
    off = content_check(backend, "Werkzeuge", (w, h, px), tool_png, "tool window")
    tr = wt.RECT()
    user32.GetWindowRect(tool, ctypes.byref(tr))
    print(f"RAIL: main {r}, tool {(tr.left, tr.top, tr.right, tr.bottom)}, "
          f"owner {user32.GetWindow(tool, 4)}, ex-style {hex(user32.GetWindowLongW(tool, -20) & 0xFFFFFFFF)}")
    if backend == "x11":
        m, t = main_x.get("jfgui Galerie"), main_x.get("Werkzeuge")
        if check(bool(m and t), "x11: the X client finds both windows", str(list(main_x))):
            check(t["transient_for"] == [m["id"]], "x11: WM_TRANSIENT_FOR = the main window",
                  f"{t['transient_for']} vs {m['id']}")
            check(t["type"] and t["type"][0] in ("_NET_WM_WINDOW_TYPE_UTILITY", "_NET_WM_WINDOW_TYPE_DIALOG"),
                  "x11: _NET_WM_WINDOW_TYPE utility (dialog under Weston)", str(t["type"]))
            weston = main_x.get("_wm", "").startswith("Weston")
            if weston and not t["state"]:
                print("INFO x11: Weston's XWM replaces _NET_WM_STATE when it maps a window (only its "
                      "own maximized / fullscreen states): SKIP_TASKBAR / SKIP_PAGER / MODAL are "
                      "set before the map but can not be read back")
            else:
                check("_NET_WM_STATE_SKIP_TASKBAR" in t["state"] and "_NET_WM_STATE_SKIP_PAGER" in t["state"],
                      "x11: _NET_WM_STATE skip taskbar / pager", str(t["state"]))
            check(len(t["hints"]) >= 9 and t["hints"][0] & 64 and t["hints"][8] == m["id"],
                  "x11: WM_HINTS window group = the main window", str(t["hints"]))
            check(len(t["motif"]) >= 3 and t["motif"][0] & 2 and t["motif"][2] == 0,
                  "x11: _MOTIF_WM_HINTS without decorations (library frame)", str(t["motif"]))
            check(t["normal_hints"] and t["normal_hints"][0] & 5 == 5,
                  "x11: WM_NORMAL_HINTS USPosition | PPosition", str(t["normal_hints"][:5]))
            want = [m["root"][0] + w_tool["rect"][0], m["root"][1] + w_tool["rect"][1]]
            check(t["root"] == want, "x11: tool window at the main client origin + (24, 48)",
                  f"{t['root']} vs {want}")
            # position round trip: the window manager moves it -> E_WINPOS -> the app keeps it
            # (a SetWindowPos of the RAIL window is not passed on to Weston: measured)
            n0 = len(app.output().splitlines())
            t1 = xprops("Werkzeuge").get("Werkzeuge", t)
            xprops("Werkzeuge", "move=40,30")
            time.sleep(1.5)
            ev = [l for l in app.output().splitlines()[n0:] if l.startswith("event type=16 ") and l.endswith("win=1")]
            t2 = xprops("Werkzeuge").get("Werkzeuge", {})
            check(t2.get("root") == [t1["root"][0] + 40, t1["root"][1] + 30],
                  "x11: the tool window moved (ConfigureWindow like a window manager)", str(t2.get("root")))
            user32.GetWindowRect(tool, ctypes.byref(tr))
            check(bool(ev) and t2.get("root") and ev[-1].split()[4:6] == [f"x={t2['root'][0]}", f"y={t2['root'][1]}"],
                  "x11: moved by the window manager -> E_WINPOS with the new position",
                  f"{ev[-1] if ev else 'no E_WINPOS'} / X {t2.get('root')}")
            # a docked tool window (centre inside the main client area) follows the main window
            # (by the distance the window manager really moved it)
            m1 = xprops("jfgui Galerie").get("jfgui Galerie", {})
            xprops("jfgui Galerie", "wmmove=60,45")
            time.sleep(1.5)
            both = xprops("jfgui Galerie", "Werkzeuge")
            m2, t3 = both.get("jfgui Galerie", {}), both.get("Werkzeuge", {})
            ok = bool(m1.get("root") and m2.get("root") and t2.get("root") and t3.get("root"))
            mdx = m2["root"][0] - m1["root"][0] if ok else 0
            mdy = m2["root"][1] - m1["root"][1] if ok else 0
            if ok and (mdx, mdy) == (0, 0):
                print("INFO x11: the window manager ignores _NET_MOVERESIZE_WINDOW (WSLg Weston): "
                      "a main-window move can not be simulated, docked following not checked here")
            else:
                check(ok and t3["root"] == [t2["root"][0] + mdx, t2["root"][1] + mdy],
                      "x11: docked tool window moves with the main window",
                      f"main moved by {(mdx, mdy)}, tool {t2.get('root')} -> {t3.get('root')}")
    else:
        out = app.output()
        sp = [l for l in out.splitlines() if "set_parent" in l]
        check(bool(sp), "wayland: xdg_toplevel.set_parent(main) sent", sp[0].strip()[:100] if sp else "no trace")
    # the close button of the tool window hides only it
    tw = tr.right - tr.left
    margin = off[0] if off else (32 if backend == "x11" else 0)
    mt = off[1] if off else margin
    for _ in range(3):
        rail_click(tool, margin + w_tool["size"][0] - 20, mt + 14)
        time.sleep(1.5)
        if not user32.IsWindowVisible(tool) or not wslcap.windows_with_title("Werkzeuge"):
            break
    check(not user32.IsWindowVisible(tool) or not wslcap.windows_with_title("Werkzeuge"),
          f"{backend}: close button hides the tool window")
    check(bool(user32.IsWindowVisible(hwnd)), f"{backend}: the main window stays")
    # modal dialog window
    dx, dy = b_dlg["r"][0] + b_dlg["r"][2] // 2, b_dlg["r"][1] + b_dlg["r"][3] // 2
    for _ in range(3):
        rail_click(hwnd, ox + dx, oy + dy)
        dlg = rail_by_title("Bild bearbeiten", old_dlg, 5)
        if dlg:
            break
    if not check(dlg is not None, f"{backend}: dialog window \"Bild bearbeiten\" appears"):
        return
    time.sleep(1.0)
    avoid_cursor(dlg, "Bild bearbeiten", backend)
    if backend == "x11":
        d = xprops("jfgui Galerie", "Bild bearbeiten")
        m, t = d.get("jfgui Galerie"), d.get("Bild bearbeiten")
        if check(bool(m and t), "x11: the X client finds the dialog"):
            check(t["transient_for"] == [m["id"]] and t["type"][:1] == ["_NET_WM_WINDOW_TYPE_DIALOG"]
                  and ("_NET_WM_STATE_MODAL" in t["state"] or (not t["state"] and d.get("_wm", "").startswith("Weston"))),
                  "x11: dialog: transient for the main window, type DIALOG, state MODAL",
                  f"{t['transient_for']} {t['type']} {t['state']}")
    time.sleep(1.5)
    w, h, px, _ = wait_content(dlg)
    wslcap.write_png(os.path.join(a.out_dir, f"x11check_windows_{backend}_dialog.png"), w, h, px)
    off = content_check(backend, "Bild bearbeiten", (w, h, px), dlg_png, "dialog window")
    if not off and backend == "x11":
        off = (32, 32)                          # WSLg: the 32 px margin of undecorated X11 windows
    if off:
        for _ in range(3):
            rail_click(dlg, off[0] + cancel["r"][0] + cancel["r"][2] // 2, off[1] + cancel["r"][1] + cancel["r"][3] // 2)
            time.sleep(1.5)
            if not user32.IsWindowVisible(dlg) or not wslcap.windows_with_title("Bild bearbeiten"):
                break
        check(not user32.IsWindowVisible(dlg) or not wslcap.windows_with_title("Bild bearbeiten"),
              f"{backend}: \"Abbrechen\" closes the dialog window")


def top_windows():
    """visible top-level windows of the desktop: {hwnd: rect}"""
    out = {}

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(h, _):
        if user32.IsWindowVisible(h):
            r = wt.RECT()
            user32.GetWindowRect(h, ctypes.byref(r))
            out[h] = (r.left, r.top, r.right, r.bottom)
        return True
    user32.EnumWindows(cb, 0)
    return out


def check_popups(a):
    """(U3) the small tool window "Popups" of the gallery (--popups-window): its menu does
    not fit into it and appears as an own window (X11 override-redirect / Wayland
    xdg_popup, each a WSLg RAIL window) below the button; a click on a row reaches the
    tool window (its event log shows the press below the tool window's client area) and
    closes the menu"""
    backend = a.backend
    print(f"--- popup windows on {backend}")
    exe = a.exe if a.exe and "gallery" in a.exe else a.bindir + "/gallery"
    ref_png = "build/x11check_ref_popups.png"
    env_h = "jfgui_CSD=1" if backend == "wayland" else ""
    recs, rc = headless_linux(exe, "size 1000 800\ntheme light\ncmd 9104\n"
                              f"shot {ref_png} window 7760\nclick #7761\ndump\n",
                              name="x11check_popups.uis", env=env_h)
    menu = first(recs, lambda r: r.get("id") == 7761 and r.get("w") == 7760, True)
    row = first(recs, lambda r: r.get("id") == 7779 and r.get("w") == 7760, True)
    first_row = first(recs, lambda r: r.get("id") == 7770 and r.get("w") == 7760, True)
    w_pop = first(recs, lambda r: r.get("window") == 7760, True)
    if not check(rc == 0 and all([menu, row, first_row, w_pop]), f"{backend}: headless reference run"):
        return
    env = {"jfgui_WLDEBUG": "1"} if backend == "wayland" else {}
    old_tool = {w[0] for w in wslcap.windows_with_title("Popups")}
    app = App(exe, ["--size", "1000x800", "--anim", "off", "--theme", "light", "--log-events",
                    "--popups-window", "--auto-close", "90000"], backend,
              "jfgui Galerie", env=env, log=f"build/x11check_popups_{backend}.log")
    try:
        hwnd = app.window()
        tool = rail_by_title("Popups", old_tool, 10) if hwnd else None
        if not check(tool is not None, f"{backend}: tool window \"Popups\" appears"):
            return
        time.sleep(1.5)
        avoid_cursor(tool, "Popups", backend)
        w, h, px, _ = wait_content(tool)
        off = content_check(backend, "Popups", (w, h, px), ref_png, "tool window")
        if off is None:
            off = (margin_of(w, h, px), margin_of(w, h, px))
        tr = wt.RECT()
        user32.GetWindowRect(tool, ctypes.byref(tr))
        before = top_windows()
        mx = off[0] + menu["r"][0] + menu["r"][2] // 2
        my = off[1] + menu["r"][1] + menu["r"][3] // 2
        popup = None
        for _ in range(3):
            rail_click(tool, mx, my)
            t = time.time() + 5
            while time.time() < t and popup is None:
                new = {k: v for k, v in top_windows().items() if k not in before and k != tool}
                for k, v in new.items():
                    if v[3] - v[1] > (menu["r"][3] * 3):        # taller than a few rows
                        popup = (k, v)
                time.sleep(0.2)
            if popup:
                break
        if not check(popup is not None, f"{backend}: the menu appears as its own window"):
            wslcap.write_png(os.path.join(a.out_dir, f"x11check_popups_{backend}_tool.png"),
                             *wslcap.capture(tool, False)[:3])
            return
        ph, pr = popup
        print(f"RAIL: tool {(tr.left, tr.top, tr.right, tr.bottom)}, popup {pr}")
        check(pr[3] > tr.bottom, f"{backend}: the menu reaches below the tool window", f"{pr[3]} > {tr.bottom}")
        time.sleep(1.0)
        pw, phh, ppx, _ = wait_content(ph)
        wslcap.write_png(os.path.join(a.out_dir, f"x11check_popups_{backend}_menu.png"), pw, phh, ppx)
        # a click on the row "Spiegeln": its offset from the first row, measured in the capture
        pm = margin_of(pw, phh, ppx)
        rx = pm + (row["r"][0] - first_row["r"][0]) + row["r"][2] // 2
        ry = pm + (row["r"][1] - first_row["r"][1]) + 4 + row["r"][3] // 2
        n0 = len(app.output().splitlines())
        rail_click(ph, rx, ry)
        time.sleep(1.0)
        ev = [l for l in app.output().splitlines()[n0:] if l.startswith("event type=2 ")]
        below = False
        for l in ev:
            f = dict(kv.split("=") for kv in l.split()[1:] if "=" in kv)
            if f.get("win") != "0" and int(f.get("y", 0)) > w_pop["size"][1]:
                below = True
        check(below, f"{backend}: the press on the row arrived as an event of the tool window",
              "; ".join(ev[-2:]))
        t = time.time() + 5
        while time.time() < t and user32.IsWindowVisible(ph):
            time.sleep(0.2)
        check(not user32.IsWindowVisible(ph), f"{backend}: the click on the row closes the menu")
    finally:
        app.kill()


class MonitorInfo(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("mon", wt.RECT), ("work", wt.RECT), ("flags", wt.DWORD)]


def monitor_bottom(hwnd):
    """the bottom of the work area of hwnd's monitor (screen px)"""
    user32.MonitorFromWindow.restype = wt.HANDLE
    mi = MonitorInfo()
    mi.cb = ctypes.sizeof(MonitorInfo)
    user32.GetMonitorInfoW(user32.MonitorFromWindow(hwnd, 2), ctypes.byref(mi))
    return mi.work.bottom


def wait_popup(before, owner, min_h, timeout=5):
    """a new top-level window (not owner) taller than min_h -> (hwnd, rect) or None"""
    t = time.time() + timeout
    while time.time() < t:
        for k, v in top_windows().items():
            if k not in before and k != owner and v[3] - v[1] > min_h:
                return k, v
        time.sleep(0.2)
    return None


def check_edges(a):
    """(ROADMAP 6) popups of the MAIN window leave it (module doc)"""
    backend = a.backend
    print(f"--- popups of the main window on {backend}")
    exe = a.exe if a.exe and "gallery" in a.exe else a.bindir + "/gallery"
    env_h = "jfgui_CSD=1" if backend == "wayland" else ""
    recs, rc = headless_linux(exe, "size 600 360\ntheme light\ncmd 9105\nscreen -2000 -2000 5000 5000\n"
                              "dump\nclick #7811\ndump\nkey esc\nclick #7810\ndump\n",
                              name="x11check_edges.uis", env=env_h)
    combo = first(recs, lambda r: r.get("id") == 7810)
    date = first(recs, lambda r: r.get("id") == 7811)
    rows = [r for r in recs if r.get("k") == "listitem"][-12:]
    if not check(rc == 0 and combo and date and len(rows) == 12, f"{backend}: headless reference run"):
        return
    env = {"jfgui_WLDEBUG": "1"} if backend == "wayland" else {}
    app = App(exe, ["--size", "600x360", "--anim", "off", "--theme", "light", "--log-events",
                    "--popup-edges", "--auto-close", "90000"], backend,
              "jfgui Galerie", env=env, log=f"build/x11check_edges_{backend}.log")
    try:
        hwnd = app.window()
        if not check(hwnd is not None, f"{backend}: the main window appears"):
            return
        time.sleep(1.5)
        avoid_cursor(hwnd, "jfgui Galerie", backend)
        off = content_origin(app, hwnd, backend)
        if off is None:
            w, h, px, _ = wait_content(hwnd)
            off = (margin_of(w, h, px), margin_of(w, h, px))
        r = app.rect()
        ch = 360
        # ---- the combo box's list: taller than the window, it leaves it at the bottom
        before = top_windows()
        popup = None
        for _ in range(3):
            rail_click(hwnd, off[0] + combo["r"][0] + combo["r"][2] // 2, off[1] + combo["r"][1] + combo["r"][3] // 2)
            popup = wait_popup(before, hwnd, ch // 2)
            if popup:
                break
        if check(popup is not None, f"{backend}: the combo box's list appears as its own window"):
            ph, pr = popup
            print(f"RAIL: main {r}, list {pr}")
            check(pr[3] > r[3], f"{backend}: the list reaches below the main window", f"{pr[3]} > {r[3]}")
            time.sleep(1.0)
            pw, phh, ppx, _ = wait_content(ph)
            wslcap.write_png(os.path.join(a.out_dir, f"x11check_edges_{backend}_list.png"), pw, phh, ppx)
            # the last row below the main window that is on the screen (Weston does not keep
            # popups on the screen): its offset from the first row, measured in the capture
            pm = margin_of(pw, phh, ppx)
            bottom = monitor_bottom(hwnd) - 8
            at = [(pm + (q["r"][0] - rows[0]["r"][0]) + q["r"][2] // 2, pm + (q["r"][1] - rows[0]["r"][1]) + 4 + q["r"][3] // 2)
                  for q in rows]
            on = [p for p in at if r[3] < pr[1] + p[1] < bottom]
            if not check(bool(on), f"{backend}: a row below the main window is on the screen"):
                return
            rx, ry = on[-1]
            print(f"INFO {backend}: click on the list at {rx},{ry} (screen {pr[0] + rx},{pr[1] + ry}, bottom {bottom})")
            n0 = len(app.output().splitlines())
            rail_click(ph, rx, ry)
            time.sleep(1.0)
            ev = [ln for ln in app.output().splitlines()[n0:] if ln.startswith("event type=2 ")]
            below = False
            for ln in ev:
                f = dict(kv.split("=") for kv in ln.split()[1:] if "=" in kv)
                if f.get("win") == "0" and int(f.get("y", 0)) > ch:
                    below = True
            check(below, f"{backend}: the press on the row arrived as an event of the main window", "; ".join(ev[-2:]))
            t = time.time() + 5
            while time.time() < t and user32.IsWindowVisible(ph):
                time.sleep(0.2)
            check(not user32.IsWindowVisible(ph), f"{backend}: the click on the row closes the list")
        # ---- X11 knows the work area: the calendar below its button, out of the window (the
        # window moved up first, so that the monitor has room below it)
        if backend == "x11":
            vt = user32.GetSystemMetrics(77)
            xprops("jfgui Galerie", f"wmmove=0,{vt + 40 - r[1]}")
            time.sleep(1.5)
            r = app.rect()
            off = content_origin(app, hwnd, backend) or off
            before = top_windows()
            popup = None
            for _ in range(3):
                rail_click(hwnd, off[0] + date["r"][0] + date["r"][2] // 2, off[1] + date["r"][1] + date["r"][3] // 2)
                popup = wait_popup(before, hwnd, 200)
                if popup:
                    break
            if check(popup is not None, "x11: the calendar appears as its own window"):
                ph, pr = popup
                print(f"RAIL: main {r}, calendar {pr}")
                check(pr[1] >= r[1] + off[1] + date["r"][1] + date["r"][3] and pr[3] > r[3],
                      "x11: below its button, reaching below the main window", f"{pr} main {r}")
                time.sleep(1.0)
                pw, phh, ppx, _ = wait_content(ph)
                wslcap.write_png(os.path.join(a.out_dir, "x11check_edges_x11_calendar.png"), pw, phh, ppx)
                rail_click(hwnd, off[0] + 300, off[1] + 150)     # a press in the window closes it
                t = time.time() + 5
                while time.time() < t and user32.IsWindowVisible(ph):
                    time.sleep(0.2)
                check(not user32.IsWindowVisible(ph), "x11: a press in the main window closes the calendar")
    finally:
        app.kill()


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["kritzel", "keys", "clip", "present", "grab", "dpi", "conn", "windows", "popups", "edges", "all"])
    ap.add_argument("--backend", default="x11", choices=["x11", "wayland"])
    ap.add_argument("--exe", help="the app of a single check")
    ap.add_argument("--bindir", default="build/linux", help="where Kritzel / linuxtest are (default build/linux)")
    ap.add_argument("--out-dir", default="build")
    a = ap.parse_args()
    os.chdir(ROOT)
    if a.what in ("kritzel", "all"):
        check_kritzel(a)
    if a.what in ("keys", "all"):
        check_keys(a)
    if a.what in ("clip", "all"):
        check_clip(a)
    if a.what in ("present", "all"):
        check_present(a)
    if a.what in ("grab", "all"):
        check_grab(a)
    if a.what in ("dpi", "all"):
        check_dpi(a)
    if a.what in ("conn", "all"):
        check_conn(a)
    if a.what in ("windows", "all"):
        check_windows(a)
    if a.what in ("popups", "all"):
        check_popups(a)
    if a.what in ("edges", "all"):
        check_edges(a)
    if a.what == "all":
        a.backend = "wayland"
        check_kritzel(a)
        check_keys(a)
        check_grab(a)
        check_windows(a)
        check_popups(a)
        check_edges(a)
    print(f"x11check: {len(FAILS)} failure(s)")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
