#!/usr/bin/env python3
"""dnd_x11.py -- XDND of lib/plat/linux_x11_dnd.rs both ways, against plattest --test dnd-x11.
Run inside WSL (WSLg's X server) from the repository root:

  python3 tools/platshared/dnd_x11.py [--rs build]

A raw X11 client (no Xlib) plays another program:
  1. a drag source over plattest's window (never mapped; found by its _NET_WM_NAME): XdndEnter
     with four types (XdndTypeList), XdndPosition at an accepted and a refused point (the
     XdndStatus answers), XdndDrop; it serves XdndSelection (text/uri-list, UTF-8 text,
     text/html, a 300 KB image/png through INCR) and checks XdndFinished; then a drag that
     only enters and leaves;
  2. a drop target: its own override-redirect window with XdndAware = 5; synthetic
     ButtonPress / MotionNotify / ButtonRelease events make plattest's drag_start run and
     cross the window: XdndEnter (types, XdndTypeList), XdndPosition (root point), XdndDrop,
     the content read with ConvertSelection, XdndFinished back;
  3. the same drag cancelled with a synthetic Esc: XdndLeave, no drop.
plattest checks its side (E_DRAG, E_DROP, the content, E_DRAG_END).  Exit 0 = passed.
"""
import argparse, os, queue, select, socket, struct, subprocess, sys, threading, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1


class X:
    """a minimal X11 client: requests, replies, and a queue of the events that arrive meanwhile"""

    def __init__(self):
        n = os.environ.get("DISPLAY", ":0").split(":")[-1].split(".")[0]
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.connect(f"/tmp/.X11-unix/X{n}")
        self.s.sendall(struct.pack("<BxHHHHxx", 0x6C, 11, 0, 0, 0))
        hdr = self.read(8)
        if hdr[0] != 1:
            raise RuntimeError("X11 setup refused")
        body = self.read(struct.unpack_from("<H", hdr, 6)[0] * 4)
        self.base, self.mask = struct.unpack_from("<II", body, 4)
        vlen, = struct.unpack_from("<H", body, 16)
        nfmt = body[21]
        o = 32 + (vlen + 3) // 4 * 4 + nfmt * 8
        self.root, = struct.unpack_from("<I", body, o)
        self.root_visual, = struct.unpack_from("<I", body, o + 32)
        self.seq = 0
        self.next_id = 1
        self.events = []
        self.atoms = {}

    def read(self, n):
        b = b""
        while len(b) < n:
            d = self.s.recv(n - len(b))
            if not d:
                raise RuntimeError("X11 connection closed")
            b += d
        return b

    def packet(self):
        p = self.read(32)
        if p[0] == 1 or p[0] & 0x7F == 35:
            p += self.read(struct.unpack_from("<I", p, 4)[0] * 4)
        return p

    def req(self, op, data1, body):
        body += b"\0" * (-len(body) % 4)
        self.s.sendall(struct.pack("<BBH", op, data1, 1 + len(body) // 4) + body)
        self.seq = (self.seq + 1) & 0xFFFF
        return self.seq

    def reply(self, seq):
        while True:
            p = self.packet()
            if p[0] == 0:
                if struct.unpack_from("<H", p, 2)[0] == seq:
                    raise RuntimeError(f"X error {p[1]} (request {p[10]})")
                continue
            if p[0] == 1 and struct.unpack_from("<H", p, 2)[0] == seq:
                return p
            if p[0] != 1:
                self.events.append(p)

    def sync(self):
        self.reply(self.req(43, 0, b""))

    def new_id(self):
        i = self.base | (self.next_id & self.mask)
        self.next_id += 1
        return i

    def atom(self, name):
        if name not in self.atoms:
            n = name.encode()
            r = self.reply(self.req(16, 0, struct.pack("<HH", len(n), 0) + n))
            self.atoms[name] = struct.unpack_from("<I", r, 8)[0]
        return self.atoms[name]

    def event(self, pred, timeout=10.0):
        """the first queued / arriving event with pred(ev) (others are kept), None after timeout"""
        end = time.time() + timeout
        while True:
            for k, e in enumerate(self.events):
                if pred(e):
                    return self.events.pop(k)
            left = end - time.time()
            if left <= 0:
                return None
            r, _, _ = select.select([self.s], [], [], left)
            if r:
                p = self.packet()
                if p[0] not in (0, 1):
                    self.events.append(p)

    def window(self, x, y, w, h, override=True, mask=0):
        wid = self.new_id()
        vals = struct.pack("<II", 1 if override else 0, mask)
        self.req(1, 0, struct.pack("<IIhhHHHHII", wid, self.root, x, y, w, h, 0, 1, 0, 0x200 | 0x800) + vals)
        return wid

    def map(self, w):
        self.req(8, 0, struct.pack("<I", w))
        self.sync()

    def prop(self, w, atom, typ, fmt, data, mode=0):
        n = len(data) // (fmt // 8)
        self.req(18, mode, struct.pack("<IIIB3xI", w, atom, typ, fmt, n) + data)

    def get(self, w, atom, delete=False, length=1 << 20):
        r = self.reply(self.req(20, 1 if delete else 0, struct.pack("<IIIII", w, atom, 0, 0, length)))
        fmt, (typ, after, n) = r[1], struct.unpack_from("<III", r, 8)
        return typ, fmt, r[32:32 + n * (fmt // 8)]

    def send(self, dest, mask, ev):
        self.req(25, 0, struct.pack("<II", dest, mask) + ev.ljust(32, b"\0"))
        self.sync()

    def client(self, dest, win, typ, l):
        ev = struct.pack("<BBHII", 33, 32, 0, win, typ) + struct.pack("<5I", *(list(l) + [0] * 5)[:5])
        self.send(dest, 0, ev)

    def input_mask(self, w, mask):
        self.req(2, 0, struct.pack("<III", w, 0x800, mask))
        self.sync()


def find_window(x, title, parent=None, depth=3):
    """the window whose _NET_WM_NAME is title (QueryTree from the root down: a window manager
    may have put it into a frame)"""
    name = x.atom("_NET_WM_NAME")
    r = x.reply(x.req(15, 0, struct.pack("<I", parent or x.root)))
    n, = struct.unpack_from("<H", r, 16)
    kids = [struct.unpack_from("<I", r, 32 + 4 * k)[0] for k in range(n)]
    for w in kids:
        if x.get(w, name, length=64)[2] == title.encode():
            return w
    if depth > 1:
        for w in kids:
            f = find_window(x, title, w, depth - 1)
            if f:
                return f
    return 0


def is_client(x, typ):
    return lambda e: e[0] & 0x7F == 33 and struct.unpack_from("<I", e, 8)[0] == x.atom(typ)


def longs(e):
    return struct.unpack_from("<5I", e, 12)


def pattern(n):
    return bytes(((i * 7 + i // 251) & 255) for i in range(n))


def serve(x, me, content, until, incr_chunk=65536):
    """answer SelectionRequests of XdndSelection on window `me` with `content` (target atom ->
    bytes; INCR above 100000 bytes) until until(event) arrives -> that event (None: timeout)"""
    sel = x.atom("XdndSelection")
    incr = x.atom("INCR")
    targets = x.atom("TARGETS")
    while True:
        e = x.event(lambda e: e[0] & 0x7F == 30 or until(e), 10)
        if e is None or e[0] & 0x7F != 30:
            return e
        t, owner, req, sel_, target, prop = struct.unpack_from("<IIIIII", e, 4)
        if prop == 0:
            prop = target
        answered = 0
        if sel_ == sel and target == targets:
            x.prop(req, prop, 4, 32, struct.pack(f"<{len(content) + 1}I", targets, *content.keys()))
            answered = prop
        elif sel_ == sel and target in content:
            data = content[target]
            if len(data) > 100000:
                x.input_mask(req, 1 << 22)  # PropertyChange of the requestor
                x.prop(req, prop, incr, 32, struct.pack("<I", len(data)))
                notify = struct.pack("<BxHIIIII", 31, 0, t, req, sel_, target, prop)
                x.send(req, 0, notify)
                off = 0
                while True:
                    d = x.event(lambda e: e[0] & 0x7F == 28 and struct.unpack_from("<II", e, 4) == (req, prop) and e[16] == 1, 10)
                    if d is None:
                        return None
                    part = data[off:off + incr_chunk]
                    x.prop(req, prop, target, 8, part)
                    x.sync()
                    off += len(part)
                    if not part:
                        break
                x.input_mask(req, 0)
                continue
            x.prop(req, prop, target, 8, data)
            answered = prop
        x.send(req, 0, struct.pack("<BxHIIIII", 31, 0, t, req, sel_, target, answered))


def convert(x, me, target, t):
    """ConvertSelection(XdndSelection, target) into a property of `me` -> the bytes (None: refused)"""
    p = x.atom("JF_DND_TEST")
    x.req(24, 0, struct.pack("<IIIII", me, x.atom("XdndSelection"), target, p, t))
    x.sync()
    e = x.event(lambda e: e[0] & 0x7F == 31 and struct.unpack_from("<I", e, 8)[0] == me, 10)
    if e is None or struct.unpack_from("<I", e, 20)[0] == 0:
        return None
    typ, fmt, data = x.get(me, p, delete=True)
    return data


def pointer(x, win, typ, rx, ry, state, t, ex=None, ey=None):
    """a synthetic ButtonPress (4) / ButtonRelease (5) / MotionNotify (6) for window win at the
    root point rx, ry (window point ex, ey; default: the same)"""
    detail = 1 if typ in (4, 5) else 0
    ex, ey = (rx if ex is None else ex), (ry if ey is None else ey)
    ev = struct.pack("<BBHIIIIhhhhHBx", typ, detail, 0, t, x.root, win, 0, rx, ry, ex, ey, state, 1)
    x.send(win, {4: 4, 5: 8, 6: 0x40}[typ], ev)


def run(rs):
    x = X()
    env = dict(os.environ, jfgui_BACKEND="x11")
    p = subprocess.Popen([str(Path(rs) / "linux" / "plattest"), "--test", "dnd-x11"], cwd=ROOT, env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    out = []

    def until(what):
        while True:
            ln = p.stdout.readline()
            if not ln:
                return False
            out.append(ln.rstrip("\n"))
            if what in ln:
                return True

    # our window: the source of part 1, the target of parts 2 and 3 (mapped at 300, 300)
    me = x.window(300, 300, 200, 150, True, (1 << 22))
    x.prop(me, x.atom("XdndAware"), 4, 32, struct.pack("<I", 5))
    x.map(me)
    copy, uri, utf8, html, png = (x.atom(n) for n in ("XdndActionCopy", "text/uri-list", "text/plain;charset=utf-8",
                                                      "text/html", "image/png"))
    try:
        if not until("dnd-x11: window"):
            check("plattest started", False, "\n".join(out[-10:]))
            return
        win = find_window(x, "plattest dnd")
        check("plattest's window found by its title", win != 0)
        # ---- 1. we drag four types over it and drop
        types = [uri, utf8, html, png]
        x.prop(me, x.atom("XdndTypeList"), 4, 32, struct.pack("<4I", *types))
        x.req(22, 0, struct.pack("<III", me, x.atom("XdndSelection"), 0))  # SetSelectionOwner
        x.sync()
        x.client(win, win, x.atom("XdndEnter"), [me, 5 << 24 | 1, uri, utf8, html])
        status = []
        for (px, py) in ((20, 30), (200, 30), (20, 30)):
            x.client(win, win, x.atom("XdndPosition"), [me, 0, px << 16 | py, 1000, copy])
            e = x.event(is_client(x, "XdndStatus"))
            status.append(longs(e) if e else None)
        check("XdndStatus at 20,30: accepted, positions wanted, action copy",
              status[0] is not None and status[0][0] == win and status[0][1] == 3 and status[0][4] == copy, status)
        check("XdndStatus at 200,30: refused", status[1] is not None and status[1][1] & 1 == 0 and status[1][4] == 0, status)
        check("XdndStatus back at 20,30: accepted", status[2] is not None and status[2][1] & 1 == 1, status)
        x.client(win, win, x.atom("XdndDrop"), [me, 0, 1001])
        content = {uri: b"file:///tmp/jf%20a.txt\r\nhttps://example.org/x\r\n", utf8: "Grüße aus X11".encode(),
                   html: b"<b>fett</b>", png: pattern(300000)}
        fin = serve(x, me, content, is_client(x, "XdndFinished"))
        check("XdndFinished: success, copy", fin is not None and longs(fin)[:3] == (win, 1, copy), fin and longs(fin))
        # a drag that only enters and leaves
        x.client(win, win, x.atom("XdndEnter"), [me, 5 << 24, utf8, 0, 0])
        x.client(win, win, x.atom("XdndPosition"), [me, 0, 20 << 16 | 30, 1002, copy])
        x.event(is_client(x, "XdndStatus"))
        x.client(win, win, x.atom("XdndLeave"), [me])
        # ---- 2. plattest drags onto our window
        if until("dnd-x11: ready to drag"):
            pointer(x, win, 4, 5, 5, 0, 2000)
            if until("dnd-x11: dragging"):
                pointer(x, win, 6, 350, 350, 0x100, 2010)
                enter = x.event(is_client(x, "XdndEnter"))
                check("XdndEnter from plattest's drag: version 5, more than three types",
                      enter is not None and longs(enter)[1] >> 24 == 5 and longs(enter)[1] & 1 == 1, enter and longs(enter))
                src = longs(enter)[0] if enter else 0
                tl = x.get(src, x.atom("XdndTypeList"))[2] if src else b""
                tl = set(struct.unpack(f"<{len(tl) // 4}I", tl)) if tl else set()
                check("XdndTypeList: uri-list, UTF-8 text, HTML, PNG", {uri, utf8, html, png} <= tl, tl)
                pos = x.event(is_client(x, "XdndPosition"))
                check("XdndPosition at the root point 350,350", pos is not None and longs(pos)[2] == 350 << 16 | 350,
                      pos and longs(pos))
                x.client(src, src, x.atom("XdndStatus"), [me, 1, 0, 0, copy])
                pointer(x, win, 5, 350, 350, 0x100, 2020)
                drop = x.event(is_client(x, "XdndDrop"))
                check("XdndDrop after the release", drop is not None and longs(drop)[0] == src, drop and longs(drop))
                t = longs(drop)[2] if drop else 0
                got = {n: convert(x, me, a, t) for n, a in (("uri", uri), ("text", utf8), ("html", html), ("png", png))}
                check("its text/uri-list", got["uri"] == b"file:///tmp/jf%20b.txt\r\n", got["uri"])
                check("its UTF-8 text", got["text"] == b"jfgui-Text", got["text"])
                check("its HTML", got["html"] == b"<i>kursiv</i>", got["html"])
                check("its PNG", bool(got["png"]) and got["png"].startswith(b"\x89PNG"), got["png"] and got["png"][:8])
                x.client(src, src, x.atom("XdndFinished"), [me, 1, copy])
        # ---- 3. the same, cancelled with Esc
        if until("dnd-x11: ready to drag again"):
            pointer(x, win, 4, 5, 5, 0, 3000)
            if until("dnd-x11: dragging again"):
                pointer(x, win, 6, 360, 360, 0x100, 3010)
                enter = x.event(is_client(x, "XdndEnter"))
                src = longs(enter)[0] if enter else 0
                x.event(is_client(x, "XdndPosition"))
                x.client(src, src, x.atom("XdndStatus"), [me, 1, 0, 0, copy])
                key = struct.pack("<BBHIIIIhhhhHBx", 2, 9, 0, 3020, x.root, win, 0, 5, 5, 5, 5, 0x100, 1)
                x.send(win, 1, key)  # KeyPress Escape (keycode 9)
                leave = x.event(is_client(x, "XdndLeave"))
                check("Esc: XdndLeave, no drop", leave is not None and not x.event(is_client(x, "XdndDrop"), 0.5), leave)
        until("\0")
    finally:
        err = p.stderr.read()
        try:
            code = p.wait(30)
        except subprocess.TimeoutExpired:
            p.kill()
            code = -1
        print("\n".join(out))
        check("plattest --test dnd-x11 passed", code == 0, err.strip()[-600:])


def run_app(rs):
    """--app: the real dndtest window (shown for a few seconds): another program's text dropped
    onto its text target (the UI's hook answers XdndStatus per target), a button's drag leaving
    the window to us, a fruit dragged out and back onto a basket (the app's own kind returns)"""
    x = X()
    env = dict(os.environ, jfgui_BACKEND="x11", jfgui_DNDTEST_PRINT="1")
    p = subprocess.Popen([str(Path(rs) / "linux" / "dndtest"), "--auto-close", "60000", "--theme", "light"], cwd=ROOT,
                         env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    lines = []
    q = queue.Queue()
    threading.Thread(target=lambda: [q.put(ln.rstrip("\n")) for ln in p.stdout], daemon=True).start()

    def until(what, timeout=10):
        end = time.time() + timeout
        while time.time() < end:
            try:
                ln = q.get(timeout=max(0.01, end - time.time()))
            except queue.Empty:
                break
            lines.append(ln)
            if what in ln:
                return True
        return False

    me = x.window(1200, 600, 200, 150, True, (1 << 22))
    x.prop(me, x.atom("XdndAware"), 4, 32, struct.pack("<I", 5))
    x.map(me)
    copy, utf8, png = x.atom("XdndActionCopy"), x.atom("text/plain;charset=utf-8"), x.atom("image/png")
    try:
        win = 0
        for _ in range(100):
            win = find_window(x, "jfgui Drag & Drop")
            if win:
                break
            time.sleep(0.1)
        check("app: the dndtest window", win != 0)
        if not win:
            return
        time.sleep(1.0)  # mapped and drawn
        r = x.reply(x.req(40, 0, struct.pack("<IIhh", win, x.root, 0, 0)))  # TranslateCoordinates
        wx, wy = struct.unpack_from("<hh", r, 12)
        print(f"  app: window at {wx},{wy}")
        # ---- another program's text: the text target takes it, an image is refused there
        x.prop(me, x.atom("XdndTypeList"), 4, 32, struct.pack("<I", utf8))
        x.req(22, 0, struct.pack("<III", me, x.atom("XdndSelection"), 0))
        x.sync()
        x.client(win, win, x.atom("XdndEnter"), [me, 5 << 24, png, 0, 0])
        x.client(win, win, x.atom("XdndPosition"), [me, 0, (wx + 82) << 16 | (wy + 229), 0, copy])
        st = x.event(is_client(x, "XdndStatus"))
        check("app: an image over the text target: refused", st is not None and longs(st)[1] & 1 == 0, st and longs(st))
        x.client(win, win, x.atom("XdndLeave"), [me])
        x.client(win, win, x.atom("XdndEnter"), [me, 5 << 24, utf8, 0, 0])
        x.client(win, win, x.atom("XdndPosition"), [me, 0, (wx + 82) << 16 | (wy + 229), 0, copy])
        st = x.event(is_client(x, "XdndStatus"))
        check("app: text over the text target: accepted (copy)", st is not None and longs(st)[1] & 1 == 1 and longs(st)[4] == copy,
              st and longs(st))
        x.client(win, win, x.atom("XdndDrop"), [me, 0, 0])
        fin = serve(x, me, {utf8: "von außen".encode()}, is_client(x, "XdndFinished"))
        check("app: XdndFinished: success", fin is not None and longs(fin)[1] == 1, fin and longs(fin))
        check("app: the text target got it", until("Abgelegt: Text -> Text-Ziel (kopieren)"), lines[-3:])
        # ---- the button "Text ziehen" (60, 300) dragged out onto our window
        pointer(x, win, 4, wx + 60, wy + 300, 0, 0, 60, 300)
        for (dx, dy) in ((70, 300), (90, 300), (1300 - wx, 660 - wy)):
            pointer(x, win, 6, wx + dx, wy + dy, 0x100, 0, dx, dy)
            time.sleep(0.15)
        enter = x.event(is_client(x, "XdndEnter"))
        check("app: the button's drag left the window: XdndEnter", enter is not None, "")
        src = longs(enter)[0] if enter else 0
        pos = x.event(is_client(x, "XdndPosition"))
        if pos:
            x.client(src, src, x.atom("XdndStatus"), [me, 1, 0, 0, copy])
        time.sleep(0.2)
        pointer(x, win, 5, 1300, 660, 0x100, 0, 1300 - wx, 660 - wy)
        drop = x.event(is_client(x, "XdndDrop"))
        got = convert(x, me, utf8, longs(drop)[2]) if drop else None
        check("app: its text arrived", got == "Hallo aus jfgui".encode(), got)
        if drop:
            x.client(src, src, x.atom("XdndFinished"), [me, 1, copy])
        check("app: DS_END with copy", until("Ende: Text ziehen kopieren"), lines[-3:])
        # ---- a fruit out of the window and back onto basket B (586, 114)
        pointer(x, win, 4, wx + 51, wy + 104, 0, 0, 51, 104)
        for (dx, dy) in ((60, 104), (80, 104), (1300 - wx, 660 - wy)):
            pointer(x, win, 6, wx + dx, wy + dy, 0x100, 0, dx, dy)
            time.sleep(0.15)
        enter = x.event(is_client(x, "XdndEnter"))
        check("app: the apple left the window", enter is not None, "")
        x.event(is_client(x, "XdndPosition"))
        pointer(x, win, 6, wx + 586, wy + 114, 0x100, 0, 586, 114)  # back over basket B
        x.event(is_client(x, "XdndLeave"), 2)
        time.sleep(0.3)
        pointer(x, win, 5, wx + 586, wy + 114, 0x100, 0, 586, 114)
        check("app: dropped back onto basket B with its own kind", until("Abgelegt: Apfel -> Korb B"), lines[-3:])
        check("app: the drag ended", until("Ende: Apfel"), lines[-3:])
    finally:
        p.kill()
        p.wait(10)
        print("\n".join("  app| " + ln for ln in lines))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    ap.add_argument("--app", action="store_true", help="also the real dndtest window (shown for a few seconds)")
    a = ap.parse_args()
    disp = os.environ.get("DISPLAY", ":0").split(":")[-1].split(".")[0]
    if not os.path.exists(f"/tmp/.X11-unix/X{disp}"):
        print("SKIP dnd_x11: no X server")
        return 0
    run(a.rs)
    if a.app:
        run_app(a.rs)
    print("dnd_x11:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
