#!/usr/bin/env python3
"""Checks of real Win32 windows made by the library (plat_win_*: tool windows, dialogs).

  python tools/wincheck.py windows [--exe build/win/gallery.exe] [--out-dir build]
  python tools/wincheck.py popups  [--exe build/win/gallery.exe] [--out-dir build]   (U3)
  python tools/wincheck.py edges   [--exe build/win/gallery.exe] [--out-dir build]   (ROADMAP 6)

Drives apps/gallery ("Fenster" section) only by POSTING / SENDING messages to the app's
own windows (never SendInput: the real mouse and keyboard are not touched):
  * a headless run of the same exe (--script with `dump` / `shot ... window ID`) gives the
    widget coordinates and the expected pixels,
  * the real app is started with --log-events; its windows are found by process id,
  * tool window "Werkzeuge": owned by the main window, WS_EX_TOOLWINDOW (no taskbar
    button, not in Alt+Tab), placed at the main client origin + (24, 48) DIP, captured
    (PrintWindow) and compared with the headless framebuffer (below the title bar),
    WM_NCHITTEST (title bar = HTCAPTION, close button = HTCLIENT), WM_MOUSEACTIVATE
    (MA_NOACTIVATE on a slider, MA_ACTIVATE on a text box), the main window keeps its
    active caption (GetWindowInfo) while another window of the app is activated and loses
    it when the app does (WM_ACTIVATEAPP), SetWindowPos -> E_WINPOS in the event log and
    the window stays there, its close button hides only the tool window, reopened at the
    moved position,
  * modal dialog "Bild bearbeiten": owned, centred on the main window, IsWindowEnabled of
    the main and the tool window 0 while it is open, "Abbrechen" closes it and enables
    them again,
  * WM_CLOSE to the main window ends the app with exit code 0.
`edges`: the popups of the MAIN window (gallery --popup-edges): the calendar leaves the window
as a popup window owned by it, below its button while the work area has room, above it with
the window in the work area's bottom right corner; the context menu there opens up and to the
left; a press on a day outside the window reaches the main window; a press in the window and
losing the keyboard close the popups.  The test moves only the app's own window.
Exit code 0 = all checks passed.  Captures go to --out-dir (they show only the app).
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import subprocess
import testenv  # noqa: F401  (keeps PaintCraft away from the user's settings)
import sys
import time

user32 = ctypes.windll.user32
try:
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass
user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
user32.GetWindow.restype = wt.HWND
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.SendMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.SendMessageW.restype = ctypes.c_ssize_t
user32.SetWindowPos.argtypes = [wt.HWND, wt.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, wt.UINT]

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import wincap  # noqa: E402  (capture, write_png)

WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEWHEEL = 0x200, 0x201, 0x202, 0x20A
WM_CLOSE, WM_NCHITTEST, WM_MOUSEACTIVATE, WM_NCACTIVATE, WM_ACTIVATEAPP = 0x10, 0x84, 0x21, 0x86, 0x1C
HTCLIENT, HTCAPTION = 1, 2
MA_ACTIVATE, MA_NOACTIVATE = 1, 3
WS_EX_TOOLWINDOW, WS_EX_APPWINDOW = 0x80, 0x40000
GW_OWNER = 4
SIZE = (1000, 800)
FAILS = []


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  (" + detail + ")") if detail else ""), flush=True)
    if not ok:
        FAILS.append(name)
    return ok


class WINDOWINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("rcWindow", wt.RECT), ("rcClient", wt.RECT),
                ("dwStyle", wt.DWORD), ("dwExStyle", wt.DWORD), ("dwWindowStatus", wt.DWORD),
                ("cxWindowBorders", wt.UINT), ("cyWindowBorders", wt.UINT),
                ("atomWindowType", wt.ATOM), ("wCreatorVersion", wt.WORD)]


def active_caption(h):
    wi = WINDOWINFO()
    wi.cbSize = ctypes.sizeof(WINDOWINFO)
    user32.GetWindowInfo(h, ctypes.byref(wi))
    return wi.dwWindowStatus & 1


def proc_windows(pid):
    out = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(h, _):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid:
            cls = ctypes.create_unicode_buffer(64)
            user32.GetClassNameW(h, cls, 64)
            if cls.value.lower() == "jfgui":  # window classes ignore case ("JFGUI")
                out.append(h)
        return True
    user32.EnumWindows(cb, 0)
    return out


def title(h):
    n = user32.GetWindowTextLengthW(h)
    b = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(h, b, n + 1)
    return b.value


def find(pid, name, timeout=5.0, visible=True):
    t = time.time() + timeout
    while time.time() < t:
        for h in proc_windows(pid):
            if title(h) == name and (not visible or user32.IsWindowVisible(h)):
                return h
        time.sleep(0.1)
    return None


def client(h):
    """client origin (screen) and size"""
    pt = wt.POINT(0, 0)
    user32.ClientToScreen(h, ctypes.byref(pt))
    rc = wt.RECT()
    user32.GetClientRect(h, ctypes.byref(rc))
    return pt.x, pt.y, rc.right, rc.bottom


def dpi_of(h):
    try:
        return user32.GetDpiForWindow(h) or 96
    except Exception:
        return 96


def lp(x, y):
    return ((y & 0xFFFF) << 16) | (x & 0xFFFF)


def click(h, x, y):
    # back to back: the app's TrackMouseEvent sees the real pointer elsewhere and posts
    # WM_MOUSELEAVE, which must come after the click
    user32.PostMessageW(h, WM_MOUSEMOVE, 0, lp(x, y))
    user32.PostMessageW(h, WM_LBUTTONDOWN, 1, lp(x, y))
    user32.PostMessageW(h, WM_LBUTTONUP, 0, lp(x, y))
    time.sleep(0.6)


def center(rec, scale=1.0):
    x, y, w, h = rec["r"]
    return int((x + w / 2) * scale), int((y + h / 2) * scale)


def read_png(path):
    import struct
    import zlib
    data = open(path, "rb").read()
    pos, w, h, idat = 8, 0, 0, b""
    while pos < len(data):
        n, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if typ == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
            bpp = {2: 3, 6: 4}[ctype]
        elif typ == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = bytearray(w * h * 3)
    prev = bytearray(stride)
    i = 0
    for y in range(h):
        f = raw[i]
        line = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        for x in range(w):
            out[(y * w + x) * 3:(y * w + x) * 3 + 3] = line[x * bpp:x * bpp + 3]
        prev = line
    return w, h, out


def compare(cap, ref_png, y0):
    """captured BGRA vs headless RGB png from row y0 down: fraction of equal pixels"""
    cw, ch, cpx = cap
    rw, rh, rpx = read_png(ref_png)
    if (cw, ch) != (rw, rh):
        return 0.0, f"size {cw}x{ch} vs {rw}x{rh}"
    same = total = 0
    for y in range(y0, ch):
        for x in range(cw):
            o = (y * cw + x) * 4
            r = (y * rw + x) * 3
            total += 1
            if cpx[o + 2] == rpx[r] and cpx[o + 1] == rpx[r + 1] and cpx[o] == rpx[r + 2]:
                same += 1
    return same / max(total, 1), f"{same}/{total} pixels equal"


def run_script(exe, out_dir, body):
    script = os.path.join(out_dir, "wincheck_ref.uis")
    with open(script, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    r = subprocess.run([exe, "--anim", "off", "--script", script], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", cwd=ROOT)
    recs = []
    for line in r.stdout.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                recs.append(json.loads(line))
            except ValueError:
                pass
    return recs, r.returncode, r.stdout


def find_scroll(exe, out_dir):
    """wheel amount (headless search) that brings the "Fenster" section into view"""
    lo, hi = -240000, 0                         # dy: more negative = further down
    for _ in range(20):
        mid = (lo + hi) // 2
        recs, rc, _ = run_script(exe, out_dir, f"size {SIZE[0]} {SIZE[1]}\ntheme light\n"
                                               f"wheel 500,400 {mid}\ndump\n")
        b = pick(recs, lambda r: r.get("t") == "Werkzeugfenster")
        if not b:
            return None
        y = b["r"][1]
        if 150 <= y <= SIZE[1] - 250:
            return mid
        if y > SIZE[1] - 250:
            hi = mid
        else:
            lo = mid
    return None


def headless(exe, out_dir, dy):
    """coordinates and reference images from a headless run of the same steps"""
    tool_png = os.path.join(out_dir, "wincheck_ref_tool.png").replace("\\", "/")
    dlg_png = os.path.join(out_dir, "wincheck_ref_dialog.png").replace("\\", "/")
    recs, rc, _ = run_script(exe, out_dir,
                             f"size {SIZE[0]} {SIZE[1]}\ntheme light\nwheel 500,400 {dy}\ndump\n"
                             f"click \"Werkzeugfenster\"\nexpect window 7600 open\n"
                             f"shot {tool_png} window 7600\n"
                             f"click \"Dialogfenster…\"\nexpect window 7620 open\n"
                             f"shot {dlg_png} window 7620\ndump\n")
    return recs, tool_png, dlg_png, rc


def pick(recs, pred, last=False):
    hits = [r for r in recs if pred(r)]
    return (hits[-1] if last else hits[0]) if hits else None


def check_windows(a):
    exe = os.path.abspath(a.exe)
    os.makedirs(a.out_dir, exist_ok=True)
    dy = find_scroll(exe, a.out_dir)
    if not check(dy is not None, "headless: wheel amount that shows the \"Fenster\" section", str(dy)):
        return
    recs, tool_png, dlg_png, rc = headless(exe, a.out_dir, dy)
    if not check(rc == 0 and bool(recs), "headless reference run"):
        return
    b_tool = pick(recs, lambda r: r.get("t") == "Werkzeugfenster")
    b_dlg = pick(recs, lambda r: r.get("t") == "Dialogfenster…")
    w_tool = pick(recs, lambda r: r.get("window") == 7600, True)
    w_dlg = pick(recs, lambda r: r.get("window") == 7620, True)
    slider = pick(recs, lambda r: r.get("id") == 7601 and r.get("w") == 7600, True)
    textbox = pick(recs, lambda r: r.get("id") == 7602 and r.get("w") == 7600, True)
    cancel = pick(recs, lambda r: r.get("t") == "Abbrechen" and r.get("w") == 7620, True)
    # the tool window's close button (where the style puts it: right end, or left with
    # SM_CAP_LEFT)
    closeb = pick(recs, lambda r: r.get("id") == 0xFFFF7811 and r.get("w") == 7600, True)
    if not check(all([b_tool, b_dlg, w_tool, w_dlg, slider, textbox, cancel, closeb]), "headless coordinates",
                 "Werkzeugfenster / Dialogfenster / windows 7600 7620 / #7601 #7602 / Abbrechen / close"):
        return
    log = os.path.join(a.out_dir, "wincheck.log")
    logf = open(log, "w", encoding="utf-8")
    proc = subprocess.Popen([exe, "--size", f"{SIZE[0]}x{SIZE[1]}", "--anim", "off", "--theme", "light",
                             "--log-events"], stdout=logf, stderr=subprocess.STDOUT, cwd=ROOT)
    try:
        run_checks(a, proc, log, dy, b_tool, b_dlg, w_tool, w_dlg, slider, textbox, cancel, tool_png, dlg_png,
                   closeb)
    finally:
        if proc.poll() is None:
            proc.kill()
        logf.close()


def events(log):
    try:
        return open(log, encoding="utf-8", errors="replace").read().splitlines()
    except OSError:
        return []


def run_checks(a, proc, log, dy, b_tool, b_dlg, w_tool, w_dlg, slider, textbox, cancel, tool_png, dlg_png,
               closeb):
    main = find(proc.pid, "jfgui Galerie", 10)
    if not check(main is not None, "main window appears"):
        return
    time.sleep(0.8)
    s = dpi_of(main) / 96
    mx, my, mw, mh = client(main)
    check((mw, mh) == (round(SIZE[0] * s), round(SIZE[1] * s)), "main window client size", f"{mw}x{mh}")
    # scroll the page to the "Fenster" section (move first: the wheel goes to the hot widget)
    user32.PostMessageW(main, WM_MOUSEMOVE, 0, lp(int(500 * s), int(400 * s)))
    user32.PostMessageW(main, WM_MOUSEWHEEL, (dy & 0xFFFF) << 16, lp(mx + int(500 * s), my + int(400 * s)))
    time.sleep(0.6)
    # ---- tool window
    click(main, *center(b_tool, s))
    tool = find(proc.pid, "Werkzeuge", 5)
    if not check(tool is not None, "tool window appears (click on \"Werkzeugfenster\")"):
        return
    time.sleep(0.5)
    ex = user32.GetWindowLongPtrW(tool, -20) & 0xFFFFFFFF
    check(user32.GetWindow(tool, GW_OWNER) == main, "tool window is owned by the main window")
    check(bool(ex & WS_EX_TOOLWINDOW) and not ex & WS_EX_APPWINDOW,
          "tool window: WS_EX_TOOLWINDOW (no taskbar button, not in Alt+Tab)", hex(ex))
    tx, ty, tw, th = client(tool)
    ts = dpi_of(tool) / 96
    ex_x, ex_y = mx + round(w_tool["rect"][0] * s), my + round(w_tool["rect"][1] * s)
    check((tx, ty) == (ex_x, ex_y), "tool window placed at the main client origin + (24, 48) DIP",
          f"{(tx, ty)} expected {(ex_x, ex_y)}")
    check((tw, th) == (round(w_tool["size"][0] * ts), round(w_tool["size"][1] * ts)),
          "tool window size = headless framebuffer", f"{tw}x{th} vs {w_tool['size']}")
    print(f"dpi: main {dpi_of(main)}, tool {dpi_of(tool)} (all monitors of this machine: see DPI note)")
    cap = wincap.capture(tool)
    wincap.write_png(os.path.join(a.out_dir, "wincheck_tool.png"), *cap)
    frac, detail = compare(cap, tool_png, int(30 * ts))
    check(frac >= 0.995, "tool window content = headless render (below the title bar)", detail)
    # non-client hit tests (title bar drags the window; the close button is ours)
    r = user32.SendMessageW(tool, WM_NCHITTEST, 0, lp(tx + tw // 2, ty + int(14 * ts)))
    check(r == HTCAPTION, "tool title bar: WM_NCHITTEST = HTCAPTION", str(r))
    cbx, cby = center(closeb, ts)
    r = user32.SendMessageW(tool, WM_NCHITTEST, 0, lp(tx + cbx, ty + cby))
    check(r == HTCLIENT, "tool close button: WM_NCHITTEST = HTCLIENT", str(r))
    # click activation: no activation for a slider, activation for a text box
    user32.PostMessageW(tool, WM_MOUSEMOVE, 0, lp(*center(slider, ts)))
    time.sleep(0.4)
    r = user32.SendMessageW(tool, WM_MOUSEACTIVATE, main, lp(HTCLIENT, WM_LBUTTONDOWN))
    check(r == MA_NOACTIVATE, "tool window: WM_MOUSEACTIVATE on the slider = MA_NOACTIVATE", str(r))
    user32.PostMessageW(tool, WM_MOUSEMOVE, 0, lp(*center(textbox, ts)))
    time.sleep(0.4)
    r = user32.SendMessageW(tool, WM_MOUSEACTIVATE, main, lp(HTCLIENT, WM_LBUTTONDOWN))
    check(r == MA_ACTIVATE, "tool window: WM_MOUSEACTIVATE on the text box = MA_ACTIVATE", str(r))
    # the main window keeps its active caption while another window of the app is active
    fg = user32.GetForegroundWindow()
    app_active = fg in proc_windows(proc.pid)
    user32.SendMessageW(main, WM_ACTIVATEAPP, 1, 0)
    user32.SendMessageW(main, WM_NCACTIVATE, 0, 0)          # as if the tool window got active
    check(active_caption(main) == 1, "main window stays drawn active while a tool window is active")
    user32.SendMessageW(main, WM_ACTIVATEAPP, 0, 0)          # the app loses the activation
    check(active_caption(main) == 0 and active_caption(tool) == 0,
          "every window drawn inactive when the app is deactivated")
    user32.SendMessageW(main, WM_ACTIVATEAPP, 1 if app_active else 0, 0)
    # moving the tool window: the app gets E_WINPOS (client origin) and keeps the position
    n0 = len(events(log))
    wr = wt.RECT()
    user32.GetWindowRect(tool, ctypes.byref(wr))
    user32.SetWindowPos(tool, None, wr.left + 40, wr.top + 30, 0, 0, 0x1 | 0x4 | 0x10)
    time.sleep(0.8)
    nx, ny = tx + 40, ty + 30
    ev = [e for e in events(log)[n0:] if e.startswith("event type=16 ")]
    check(any(f"x={nx} y={ny}" in e and not e.endswith("win=0") for e in ev),
          "SetWindowPos on the tool window -> E_WINPOS in the app", "; ".join(ev) or "none")
    check(client(tool)[:2] == (nx, ny), "tool window stays at the new position", str(client(tool)[:2]))
    # its close button hides only the tool window
    click(tool, cbx, cby)
    time.sleep(0.3)
    check(not user32.IsWindowVisible(tool) and user32.IsWindowVisible(main),
          "close button hides the tool window, the main window stays")
    click(main, *center(b_tool, s))
    tool2 = find(proc.pid, "Werkzeuge", 5)
    check(tool2 is not None and client(tool2)[:2] == (nx, ny),
          "tool window reopened at the moved position", str(client(tool2)[:2]) if tool2 else "")
    # docked tool windows (centre inside the main client area) move with the main window;
    # a tool window dragged out of the main window stays where it is
    if tool2 is not None:
        mr = wt.RECT()
        user32.GetWindowRect(main, ctypes.byref(mr))
        t0 = client(tool2)[:2]
        user32.SetWindowPos(main, None, mr.left + 60, mr.top + 45, 0, 0, 0x1 | 0x4 | 0x10)
        time.sleep(0.8)
        check(client(tool2)[:2] == (t0[0] + 60, t0[1] + 45), "docked tool window moves with the main window",
              f"{client(tool2)[:2]} expected {(t0[0] + 60, t0[1] + 45)}")
        tr = wt.RECT()
        user32.GetWindowRect(tool2, ctypes.byref(tr))
        user32.SetWindowPos(tool2, None, mr.left + 60 - (tr.right - tr.left) - 30, tr.top, 0, 0, 0x1 | 0x4 | 0x10)
        time.sleep(0.8)
        t1 = client(tool2)[:2]
        user32.SetWindowPos(main, None, mr.left, mr.top, 0, 0, 0x1 | 0x4 | 0x10)
        time.sleep(0.8)
        check(client(tool2)[:2] == t1, "tool window dragged out of the main window stays there",
              f"{client(tool2)[:2]} expected {t1}")
    # ---- modal dialog window
    click(main, *center(b_dlg, s))
    dlg = find(proc.pid, "Bild bearbeiten", 5)
    if not check(dlg is not None, "dialog window appears (click on \"Dialogfenster…\")"):
        return
    time.sleep(0.5)
    ex = user32.GetWindowLongPtrW(dlg, -20) & 0xFFFFFFFF
    check(user32.GetWindow(dlg, GW_OWNER) == main and not ex & WS_EX_TOOLWINDOW,
          "dialog window: owned by the main window, not a tool window", hex(ex))
    dx, dy, dw, dh = client(dlg)
    ds = dpi_of(dlg) / 96
    ex_x, ex_y = mx + round(w_dlg["rect"][0] * s), my + round(w_dlg["rect"][1] * s)
    check(abs(dx - ex_x) <= 1 and abs(dy - ex_y) <= 1, "dialog centred on the main window (WINRECT)",
          f"{(dx, dy)} expected {(ex_x, ex_y)}")
    check(not user32.IsWindowEnabled(main), "main window disabled while the modal dialog is open")
    check(tool2 is None or not user32.IsWindowEnabled(tool2), "tool window disabled while the modal dialog is open")
    cap = wincap.capture(dlg)
    wincap.write_png(os.path.join(a.out_dir, "wincheck_dialog.png"), *cap)
    frac, detail = compare(cap, dlg_png, int(34 * ds))
    check(frac >= 0.99, "dialog content = headless render (below the title bar)", detail)
    click(dlg, *center(cancel, ds))
    time.sleep(0.4)
    check(not user32.IsWindowVisible(dlg), "\"Abbrechen\" closes the dialog window")
    check(user32.IsWindowEnabled(main) and (tool2 is None or user32.IsWindowEnabled(tool2)),
          "main and tool window enabled again after the dialog")
    # ---- end
    user32.PostMessageW(main, WM_CLOSE, 0, 0)
    try:
        rc = proc.wait(8)
    except subprocess.TimeoutExpired:
        rc = None
    check(rc == 0, "WM_CLOSE ends the app with exit code 0", f"exit {rc}")


# ---------------------------------------------------------------- popups (U3)

def popup_windows(pid):
    out = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(h, _):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid:
            cls = ctypes.create_unicode_buffer(64)
            user32.GetClassNameW(h, cls, 64)
            if cls.value.lower() == "jfguipopup" and user32.IsWindowVisible(h):  # "JfGuiPopup"
                out.append(h)
        return True
    user32.EnumWindows(cb, 0)
    return out


def check_popups(a):
    """tool window "Popups" (gallery --popups-window): its menu, combo list and tooltip do
    not fit into it and appear as borderless popup windows owned by it; clicks on them
    reach the tool window's widgets (U3)"""
    exe = os.path.abspath(a.exe)
    os.makedirs(a.out_dir, exist_ok=True)
    recs, rc, _ = run_script(exe, a.out_dir, f"size {SIZE[0]} {SIZE[1]}\ntheme light\ncmd 9104\n"
                                             f"click #7761\ndump\n")
    w_pop = pick(recs, lambda r: r.get("window") == 7760, True)
    menu = pick(recs, lambda r: r.get("id") == 7761 and r.get("w") == 7760, True)
    row = pick(recs, lambda r: r.get("id") == 7779 and r.get("w") == 7760, True)   # "Spiegeln"
    combo = pick(recs, lambda r: r.get("id") == 7762 and r.get("w") == 7760, True)
    if not check(rc == 0 and all([w_pop, menu, row, combo]), "headless coordinates",
                 "window 7760, #7761, row #7779, #7762"):
        return
    log = os.path.join(a.out_dir, "wincheck_popups.log")
    logf = open(log, "w", encoding="utf-8")
    proc = subprocess.Popen([exe, "--size", f"{SIZE[0]}x{SIZE[1]}", "--anim", "off", "--theme", "light",
                             "--log-events", "--popups-window", "--auto-close", "60000"],
                            stdout=logf, stderr=subprocess.STDOUT, cwd=ROOT)
    try:
        main = find(proc.pid, "jfgui Galerie")
        tool = find(proc.pid, "Popups")
        if not check(bool(main and tool), "main window and tool window \"Popups\" shown"):
            return
        time.sleep(0.5)
        sc = dpi_of(tool) / 96.0
        tx, ty, tw, th = client(tool)
        check(not popup_windows(proc.pid), "no popup window before")
        # open the menu: a popup window below the button, beyond the tool window
        x, y = center(menu, sc)
        click(tool, x, y)
        pops = popup_windows(proc.pid)
        if check(len(pops) == 1, "the menu is a popup window", str(len(pops))):
            p = pops[0]
            px, py, pw, ph = client(p)
            owner = user32.GetWindow(p, GW_OWNER)
            ex = user32.GetWindowLongPtrW(p, -20)
            check(owner == tool, "owned by the tool window")
            check(bool(ex & WS_EX_TOOLWINDOW) and bool(ex & 0x08000000), "WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE")
            check(py + ph > ty + th, "reaches below the tool window", f"popup {px},{py} {pw}x{ph} tool {tx},{ty} {tw}x{th}")
            check(abs(py - (ty + int((menu['r'][1] + menu['r'][3]) * sc))) <= int(8 * sc),
                  "placed below the button")
            cap = wincap.capture(p)
            if cap:
                wincap.write_png(os.path.join(a.out_dir, "wincheck_popup_menu.png"), *cap)
            ma = user32.SendMessageW(p, WM_MOUSEACTIVATE, tool, 0)
            check(ma == MA_NOACTIVATE, "WM_MOUSEACTIVATE -> MA_NOACTIVATE")
            # a click on the row "Spiegeln" (popup client coordinates)
            rx, ry = center(row, sc)
            click(p, rx - (px - tx), ry - (py - ty))
            check(not popup_windows(proc.pid), "a click on a row closes the menu")
            import re as _re
            downs = [_re.search(r"type=2 .*y=(-?\d+) .*win=(\d+)", l) for l in events(log)]
            check(any(m and int(m.group(2)) != 0 and int(m.group(1)) >= th for m in downs),
                  "the row's press arrived as an event of the tool window (below its client area)")
        # the combo list: 20 rows, taller than the tool window
        x, y = center(combo, sc)
        click(tool, x, y)
        pops = popup_windows(proc.pid)
        if check(len(pops) == 1, "the combo list is a popup window", str(len(pops))):
            px, py, pw, ph = client(pops[0])
            check(ph > th, "taller than the tool window", f"{ph} > {th}")
            cap = wincap.capture(pops[0])
            if cap:
                wincap.write_png(os.path.join(a.out_dir, "wincheck_popup_combo.png"), *cap)
            # light dismiss: a press in the main window closes it
            click(main, 900, 700)
            check(not popup_windows(proc.pid), "a press in the main window closes the list")
        user32.PostMessageW(main, WM_CLOSE, 0, 0)
        try:
            code = proc.wait(10)
        except subprocess.TimeoutExpired:
            code = None
        check(code == 0, "WM_CLOSE ends the app", str(code))
    finally:
        if proc.poll() is None:
            proc.kill()
        logf.close()


# ---------------------------------------------------------------- the main window's popups (§6)

class MonitorInfo(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("mon", wt.RECT), ("work", wt.RECT), ("flags", wt.DWORD)]


def work_area(h):
    """left, top, right, bottom of the work area of h's monitor"""
    user32.MonitorFromWindow.restype = wt.HANDLE
    m = user32.MonitorFromWindow(h, 2)
    mi = MonitorInfo()
    mi.cb = ctypes.sizeof(MonitorInfo)
    user32.GetMonitorInfoW(m, ctypes.byref(mi))
    return mi.work.left, mi.work.top, mi.work.right, mi.work.bottom


def win_rect(h):
    rc = wt.RECT()
    user32.GetWindowRect(h, ctypes.byref(rc))
    return rc.left, rc.top, rc.right, rc.bottom


def rclick(h, x, y):
    user32.PostMessageW(h, WM_MOUSEMOVE, 0, lp(x, y))
    user32.PostMessageW(h, 0x204, 2, lp(x, y))      # WM_RBUTTONDOWN
    user32.PostMessageW(h, 0x205, 0, lp(x, y))      # WM_RBUTTONUP
    time.sleep(0.6)


def region_diff(a, b, x0, y0, x1, y1):
    """pixels that differ between two client captures inside x0..x1, y0..y1"""
    (w, h, pa), (_, _, pb) = a, b
    n = 0
    for y in range(max(0, y0), min(h, y1)):
        o = y * w * 4
        for x in range(max(0, x0), min(w, x1)):
            k = o + x * 4
            if pa[k:k + 3] != pb[k:k + 3]:
                n += 1
    return n


def check_edges(a):
    """(ROADMAP 6) popups of the main window leave it and stay in the work area (module doc)"""
    exe = os.path.abspath(a.exe)
    os.makedirs(a.out_dir, exist_ok=True)
    # the layout, the calendar's day "20", the context menu's rows (owner coordinates, room on
    # every side)
    recs, rc, _ = run_script(exe, a.out_dir, "size 600 400\ntheme light\ncmd 9105\nscreen -2000 -2000 5000 5000\n"
                                             "dump\nclick #7811\ndump\nkey esc\nrclick #7812\ndump\n")
    date = pick(recs, lambda r: r.get("id") == 7811)
    tip = pick(recs, lambda r: r.get("id") == 7812)
    day = pick(recs, lambda r: r.get("t") == "20" and r.get("k") == "button")
    first = pick(recs, lambda r: r.get("k") == "menuitem" and r.get("t") == "\u00d6ffnen")
    sub = pick(recs, lambda r: r.get("k") == "menuitem" and r.get("t") == "Anordnen")
    if not check(rc == 0 and all([date, tip, day, first, sub]), "headless coordinates", "#7811, #7812, day 20, menu rows"):
        return
    # "Anordnen" in the context menu's popup window (its origin: the first row less the padding)
    sub_at = ((sub["r"][0] - first["r"][0] + 4 + sub["r"][2] / 2), (sub["r"][1] - first["r"][1] + 4 + sub["r"][3] / 2))
    log = os.path.join(a.out_dir, "wincheck_edges.log")
    logf = open(log, "w", encoding="utf-8")
    proc = subprocess.Popen([exe, "--size", "600x400", "--anim", "off", "--theme", "light", "--log-events",
                             "--popup-edges", "--auto-close", "60000"], stdout=logf, stderr=subprocess.STDOUT, cwd=ROOT)
    try:
        main = find(proc.pid, "jfgui Galerie")
        if not check(bool(main), "main window shown"):
            return
        time.sleep(0.5)
        sc = dpi_of(main) / 96.0
        wl, wtop, wr, wb = work_area(main)
        l, t, r, b = win_rect(main)
        w, h = r - l, b - t

        def place(x, y):
            user32.SetWindowPos(main, 0, x, y, 0, 0, 0x15)   # NOSIZE | NOZORDER | NOACTIVATE
            time.sleep(0.5)
            return client(main)

        def inside_work(px, py, pw, ph):
            return wl <= px and px + pw <= wr and wtop <= py and py + ph <= wb

        # ---- the window in the middle of the work area: the calendar below, out of the window
        cx, cy, cw, ch = place((wl + wr - w) // 2, (wtop + wb - h) // 2)
        click(main, *center(date, sc))
        pops = popup_windows(proc.pid)
        if check(len(pops) == 1, "the calendar is a popup window", str(len(pops))):
            p = pops[0]
            px, py, pw, ph = client(p)
            check(user32.GetWindow(p, GW_OWNER) == main, "owned by the main window")
            below = cy + int((date["r"][1] + date["r"][3]) * sc)
            check(py >= below and py + ph > cy + ch, "below its button, beyond the window's bottom edge",
                  f"popup {px},{py} {pw}x{ph} window {cx},{cy} {cw}x{ch}")
            check(inside_work(px, py, pw, ph), "inside the work area")
            cap = wincap.capture(p)
            if cap:
                wincap.write_png(os.path.join(a.out_dir, "wincheck_edges_below.png"), *cap)
            # the day "20": owner coordinates -> the popup's client coordinates
            dx, dy = center(day, sc)
            click(p, dx - (px - cx), dy - (py - cy))
            check(not popup_windows(proc.pid), "a click on a day closes the calendar")
            import re as _re
            downs = [_re.search(r"type=2 .*y=(-?\d+) .*win=(\d+)", ln) for ln in events(log)]
            check(any(m and int(m.group(2)) == 0 and int(m.group(1)) >= ch for m in downs),
                  "the press arrived as an event of the main window (below its client area)")
        # ---- the window at the bottom of the work area: the calendar flips above its button
        # (it fits into the window there: drawn inside, no popup window); the context menu goes
        # out to the right, but above the pointer
        cx, cy, cw, ch = place((wl + wr - w) // 2, wb - h)
        before = wincap.capture(main)
        click(main, *center(date, sc))
        after = wincap.capture(main)
        x0, x1 = int(date["r"][0] * sc), int((date["r"][0] + 250) * sc)
        n = region_diff(before, after, x0, 0, x1, int((date["r"][1] - 4) * sc)) if before and after else 0
        check(not popup_windows(proc.pid) and n > 2000, "at the bottom edge the calendar opens above its button, inside the window",
              f"{n} pixels changed above the button")
        if after:
            wincap.write_png(os.path.join(a.out_dir, "wincheck_edges_above.png"), *after)
        click(main, int(300 * sc), int(200 * sc))
        x, y = center(tip, sc)
        rclick(main, x, y)
        pops = popup_windows(proc.pid)
        if check(len(pops) == 1, "the context menu at the bottom edge is a popup window", str(len(pops))):
            px, py, pw, ph = client(pops[0])
            check(px >= cx + x - int(4 * sc) and py + ph <= cy + y + int(4 * sc) and px + pw > cx + cw and inside_work(px, py, pw, ph),
                  "to the right of the pointer (out of the window), above it, inside the work area",
                  f"popup {px},{py} {pw}x{ph} pointer {cx + x},{cy + y} window right {cx + cw}")
        click(main, int(300 * sc), int(200 * sc))
        check(not popup_windows(proc.pid), "a press in the window closes the context menu")
        # ---- the window at the right edge: the context menu to the left of the pointer, below
        # it (out of the window); its submenu opens to the left
        cx, cy, cw, ch = place(wr - w, (wtop + wb - h) // 2)
        rclick(main, x, y)
        pops = popup_windows(proc.pid)
        if check(len(pops) == 1, "the context menu at the right edge is a popup window", str(len(pops))):
            p = pops[0]
            px, py, pw, ph = client(p)
            check(px + pw <= cx + x + int(4 * sc) and py >= cy + y - int(4 * sc) and py + ph > cy + ch and inside_work(px, py, pw, ph),
                  "to the left of the pointer, below it (out of the window), inside the work area",
                  f"popup {px},{py} {pw}x{ph} pointer {cx + x},{cy + y} window bottom {cy + ch}")
            click(p, int(sub_at[0] * sc), int(sub_at[1] * sc))
            subs = [q for q in popup_windows(proc.pid) if q != p]
            if check(len(subs) == 1, "the submenu is a second popup window", str(len(subs))):
                sx, sy, sw, sh = client(subs[0])
                check(sx + sw <= px + int(8 * sc) and inside_work(sx, sy, sw, sh), "the submenu opens to the left of the menu",
                      f"submenu {sx},{sy} {sw}x{sh} menu {px},{py}")
            # the app loses the keyboard: like the system's menus its popups close
            user32.PostMessageW(main, 0x0007, 0, 0)    # WM_SETFOCUS
            time.sleep(0.4)
            user32.PostMessageW(main, 0x0008, 0, 0)    # WM_KILLFOCUS
            time.sleep(0.6)
            check(not popup_windows(proc.pid), "losing the keyboard closes the menus")
        user32.PostMessageW(main, WM_CLOSE, 0, 0)
        try:
            code = proc.wait(10)
        except subprocess.TimeoutExpired:
            code = None
        check(code == 0, "WM_CLOSE ends the app", str(code))
    finally:
        if proc.poll() is None:
            proc.kill()
        logf.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["windows", "popups", "edges"])
    ap.add_argument("--exe", default="build/win/gallery.exe")
    ap.add_argument("--out-dir", default="build")
    a = ap.parse_args()
    if a.what == "windows":
        check_windows(a)
    if a.what == "popups":
        check_popups(a)
    if a.what == "edges":
        check_edges(a)
    print("wincheck:", "all passed" if not FAILS else f"{len(FAILS)} failed: " + ", ".join(FAILS))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
