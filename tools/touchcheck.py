#!/usr/bin/env python3
"""Real fingers on a real PaintCraft window, without a touch screen: InjectTouchInput (Windows).

  python tools/touchcheck.py [--exe build/win/PaintCraft.exe]

What a script cannot show: the system's own decisions about a contact.  Which window a contact goes to
(a wide one is given to a window frame it touches), that WM_POINTER replaces the mouse emulation, a window
that follows a finger.  The contacts are as wide as a finger (60 px) because that is what makes the
system decide differently.

It needs the desktop unlocked and nothing covering the window.  Every contact first asks WindowFromPoint
and is not made on another program's window: the run then ends with exit code 77 and a clear message, as
it does when the system refuses to inject (a locked screen, a remote session).  Exit code 0 = all checks
passed, 1 = a check failed.  The real mouse and keyboard are not touched; the windows are the test's own,
with their settings in a folder of their own.
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

user32 = ctypes.WinDLL("user32", use_last_error=True)
try:
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKIP = 77
FAILS = []


class PointerInfo(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("id", ctypes.c_uint32), ("frame", ctypes.c_uint32), ("flags", ctypes.c_uint32),
                ("dev", ctypes.c_void_p), ("hwnd", wt.HWND), ("pt", wt.POINT), ("him", wt.POINT), ("ptraw", wt.POINT),
                ("himraw", wt.POINT), ("time", wt.DWORD), ("hist", ctypes.c_uint32), ("data", ctypes.c_int32),
                ("keys", wt.DWORD), ("perf", ctypes.c_uint64), ("change", wt.DWORD)]


class TouchInfo(ctypes.Structure):
    _fields_ = [("p", PointerInfo), ("tflags", ctypes.c_uint32), ("mask", ctypes.c_uint32), ("rc", wt.RECT), ("rcraw", wt.RECT),
                ("orient", ctypes.c_uint32), ("press", ctypes.c_uint32)]


DOWN, UPDATE, UP = 0x10000 | 6, 0x20000 | 6, 0x40000
RADIUS = 30  # half a finger's contact (px)
user32.InjectTouchInput.argtypes = [ctypes.c_uint32, ctypes.POINTER(TouchInfo)]
user32.WindowFromPoint.argtypes = [wt.POINT]
user32.WindowFromPoint.restype = wt.HWND
user32.SetWindowPos.argtypes = [wt.HWND, wt.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, wt.UINT]
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetClientRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.ClientToScreen.argtypes = [wt.HWND, ctypes.POINTER(wt.POINT)]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]


class Skip(Exception):
    pass


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  (" + str(detail) + ")") if detail != "" else ""), flush=True)
    if not ok:
        FAILS.append(name)


def windows_of(pid):
    out = []

    def f(h, _):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid and user32.IsWindowVisible(h):
            out.append(h)
        return True

    user32.EnumWindows(ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)(f), 0)
    return out


def rect(h):
    r = wt.RECT()
    user32.GetWindowRect(h, ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def origin(h):
    p = wt.POINT(0, 0)
    user32.ClientToScreen(h, ctypes.byref(p))
    return p.x, p.y


def title(h):
    b = ctypes.create_unicode_buffer(200)
    user32.GetWindowTextW(h, b, 200)
    return b.value


def contact(i, flags, x, y):
    t = TouchInfo()
    t.p.type = 2
    t.p.id = i
    t.p.flags = flags
    t.p.pt = wt.POINT(x, y)
    t.mask = 1 | 2 | 4
    t.rc = wt.RECT(x - RADIUS, y - RADIUS, x + RADIUS, y + RADIUS)
    t.orient = 90
    t.press = 32000
    return t


def inject(ts):
    arr = (TouchInfo * len(ts))(*ts)
    if not user32.InjectTouchInput(len(ts), arr):
        raise Skip("InjectTouchInput refused (%d): a locked screen or a remote session?" % ctypes.get_last_error())


class App:
    def __init__(self, exe, cfg, log):
        self.log_path = log
        self.logf = open(log, "w")
        env = dict(os.environ, PaintCraft_CONFIG_DIR=cfg)
        self.p = subprocess.Popen([exe, "--auto-close", "45000", "--log-events"], env=env, stdout=self.logf, stderr=self.logf)
        self.mark_at = 0

    def windows(self):
        return windows_of(self.p.pid)

    def by_title(self, t):
        return next((h for h in self.windows() if title(h) == t), None)

    def ours(self, x, y):
        h = user32.WindowFromPoint(wt.POINT(x, y))
        if not h:
            return False
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(p))
        return p.value == self.p.pid

    def guard(self, *pts):
        for (x, y) in pts:
            if not self.ours(x, y):
                raise Skip("(%d, %d) is covered by another window" % (x, y))

    def tap(self, x, y, jitter=0):
        self.guard((x, y))
        inject([contact(0, DOWN, x, y)])
        time.sleep(0.03)
        inject([contact(0, UPDATE, x, y + jitter)])
        time.sleep(0.03)
        inject([contact(0, UP, x, y + jitter)])
        time.sleep(0.35)

    def drag(self, x0, y0, x1, y1, hold=0.0, ms=300):
        self.guard((x0, y0))
        inject([contact(0, DOWN, x0, y0)])
        end = time.time() + hold
        while time.time() < end:
            time.sleep(0.03)
            inject([contact(0, UPDATE, x0, y0)])
        n = max(1, ms // 16)
        for i in range(1, n + 1):
            time.sleep(0.016)
            inject([contact(0, UPDATE, x0 + (x1 - x0) * i // n, y0 + (y1 - y0) * i // n)])
        inject([contact(0, UP, x1, y1)])
        time.sleep(0.35)

    def two(self, a, b, da, db, ms=400):
        self.guard(a, b)
        inject([contact(0, DOWN, *a), contact(1, DOWN, *b)])
        n = max(1, ms // 16)
        for i in range(1, n + 1):
            time.sleep(0.016)
            inject([contact(0, UPDATE, a[0] + da[0] * i // n, a[1] + da[1] * i // n),
                    contact(1, UPDATE, b[0] + db[0] * i // n, b[1] + db[1] * i // n)])
        inject([contact(0, UP, a[0] + da[0], a[1] + da[1]), contact(1, UP, b[0] + db[0], b[1] + db[1])])
        time.sleep(0.35)

    def events(self):
        """the events logged since the last call: (type, btn, mods, x, y, data, win)"""
        self.logf.flush()
        with open(self.log_path) as f:
            lines = f.read().splitlines()
        new = lines[self.mark_at:]
        self.mark_at = len(lines)
        out = []
        for l in new:
            m = re.match(r"event type=(\d+) btn=(\d+) mods=(\d+) x=(-?\d+) y=(-?\d+) data=([0-9a-f]+) win=(\d+)", l)
            if m:
                out.append((int(m[1]), int(m[2]), int(m[3]), int(m[4]), int(m[5]), int(m[6], 16), int(m[7])))
        return out

    def close(self):
        for h in self.windows():
            user32.PostMessageW(h, 0x10, 0, 0)
        try:
            self.p.wait(5)
        except subprocess.TimeoutExpired:
            self.p.kill()
        self.logf.close()


def is_finger(e):
    return (e[5] >> 16) & 3 == 3


def near(e, x, y, d=8):
    return abs(e[3] - x) <= d and abs(e[4] - y) <= d


def run(exe, work):
    a = App(exe, os.path.join(work, "cfg"), os.path.join(work, "events.txt"))
    try:
        time.sleep(4)
        main = next((h for h in a.windows() if "PaintCraft" in title(h)), None)
        if main is None:
            raise Skip("no main window (no desktop?)")
        # on top of what the user has open, without taking the keyboard
        for h in a.windows():
            user32.SetWindowPos(h, wt.HWND(-1), 0, 0, 0, 0, 0x13)
        for h in a.windows():
            user32.SetWindowPos(h, wt.HWND(-2), 0, 0, 0, 0, 0x13)
        ox, oy = origin(main)
        a.events()

        # ---- the gear: a finger, no mouse emulation
        gx, gy = 1085, 24
        a.tap(ox + gx, oy + gy)
        ev = a.events()
        check(any(e[0] == 2 and is_finger(e) and near(e, gx, gy) for e in ev), "gear: the contact is a finger (WM_POINTER)")
        check(not any(e[0] in (2, 3) and not is_finger(e) and e[1] == 0 for e in ev), "gear: no raw press without a finger's data")
        check(any(e[0] == 2 and e[1] == 1 and near(e, gx, gy) for e in ev), "gear: the recogniser's press reaches the button")
        d = a.by_title("Einstellungen")
        check(d is not None, "gear: the settings window opens")
        if d:
            user32.PostMessageW(d, 0x10, 0, 0)
            time.sleep(0.5)
        a.events()

        # ---- a finger that rolls off the button while it taps
        a.tap(ox + gx, oy + 36, jitter=8)
        d = a.by_title("Einstellungen")
        check(d is not None, "gear: a tap that rolled 8 px off the button still opens it")
        if d:
            user32.PostMessageW(d, 0x10, 0, 0)
            time.sleep(0.5)
        a.events()

        # ---- the zoom slider under the panel's invisible frame
        layers = a.by_title("Ebenen")
        before = rect(layers) if layers else None
        c = wt.RECT()
        user32.GetClientRect(main, ctypes.byref(c))
        sx, sy = c.right - 72, c.bottom - 16
        a.drag(ox + sx, oy + sy, ox + sx - 50, oy + sy, ms=250)
        ev = a.events()
        check(rect(layers) == before, "slider: the panel above stays as it is", (before, rect(layers)))
        check(any(e[0] == 2 and e[1] == 1 and e[6] == 0 and near(e, sx, sy) for e in ev), "slider: the press goes to the main window, at the slider")
        check(not any(e[0] == 8 and e[6] != 0 for e in ev), "slider: no window was resized")

        # ---- a panel: a quick swipe moves nothing, after the hold every direction
        tools = next((h for h in a.windows() if title(h) == ""), None)
        if tools is None:
            raise Skip("the tool window is not there")
        r0 = rect(tools)
        tx, ty = origin(tools)
        a.drag(tx + 20, ty + 14, tx + 80, ty + 74, ms=150)
        check(rect(tools) == r0, "panel: a quick swipe on the title bar moves nothing", (r0, rect(tools)))
        a.drag(tx + 20, ty + 14, tx + 20, ty + 114, hold=1.0)
        r1 = rect(tools)
        check(r1[0] == r0[0] and abs(r1[1] - r0[1] - 100) <= 2, "panel: after the hold it follows straight down", (r0, r1))
        tx, ty = origin(tools)
        a.drag(tx + 20, ty + 14, tx + 170, ty - 46, hold=1.0)
        r2 = rect(tools)
        check(abs(r2[0] - r1[0] - 150) <= 2 and abs(r2[1] - r1[1] + 60) <= 2, "panel: ... and up and to the right", (r1, r2))
        a.events()

        # ---- two fingers: no Ctrl + wheel, no mouse
        cx, cy = 500, 400
        a.two((ox + cx - 100, oy + cy), (ox + cx + 100, oy + cy), (-80, 0), (80, 0))
        ev = a.events()
        check(not any(e[0] == 4 for e in ev), "pinch: no wheel events (the system's Ctrl + wheel)")
        ids = {e[1] for e in ev if e[0] == 1 and is_finger(e)}
        check(len(ids) == 2, "pinch: two fingers with their ids", sorted(ids))
        a.two((ox + cx - 100, oy + cy), (ox + cx + 100, oy + cy), (120, 40), (120, 40))
        ev = a.events()
        check(not any(e[0] == 4 for e in ev), "two-finger pan: no wheel events")
        check(sum(1 for e in ev if e[0] == 1 and is_finger(e)) > 10, "two-finger pan: the moves arrive as fingers")
        strays = [e for e in ev if e[0] in (1, 2, 3) and not is_finger(e) and e[1] == 0 and e[5] == 0 and near(e, cx - 100, cy, 40)]
        check(not strays, "two-finger pan: no mouse messages at the fingers", strays[:3])

        # ---- a text field: the on-screen keyboard is asked for (what it looks like only a tablet shows),
        # another widget takes it away; the program lives through both calls
        a.tap(ox + 360, oy + 108)
        time.sleep(0.5)
        a.tap(ox + 417, oy + 108)
        time.sleep(0.5)
        check(a.p.poll() is None, "text field: InputPane asked to show and to hide, the program runs on")
    finally:
        a.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=os.path.join(ROOT, "build", "win", "PaintCraft.exe"))
    args = ap.parse_args()
    if not hasattr(user32, "InitializeTouchInjection") or not user32.InitializeTouchInjection(2, 3):
        print("SKIP: this system does not inject touch input")
        return SKIP
    work = tempfile.mkdtemp(prefix="touchcheck")
    try:
        run(args.exe, work)
    except Skip as e:
        print("SKIP: %s" % e)
        return SKIP
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print("%d failure(s)" % len(FAILS))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
