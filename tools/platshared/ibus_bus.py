#!/usr/bin/env python3
"""ibus_bus.py -- plat's input method client of X11 (lib/plat/linux_ibus.rs, linux_dbus.rs)
against a fake session bus.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/ibus_bus.py [--rs build]

The fake bus (SASL EXTERNAL, little-endian messages; the marshalling of portal_bus.py) plays the
bus daemon, org.freedesktop.portal.IBus and an input method behind the input context:

  'a'        UpdatePreeditText "かな", cursor 1 (characters), visible; taken
  Return     CommitText "かな"; taken
  'z'        not taken
  BackSpace  ForwardKeyEvent Left and 'c'; taken
  'b'        UpdatePreeditText "b"; taken
  Escape     HidePreeditText; taken
  'q'        no answer at all (the client gives up after 300 ms)
  releases   not taken

plattest --test ibus-bus checks what reaches the UI thread; the fake checks what it was told
(CreateInputContext, AddMatch, SetCapabilities, FocusIn once, the caret locations, every
ProcessKeyEvent, FocusOut).  Then plattest --test ibus-none runs twice: on a bus whose portal
answers with an error, and with an unreachable bus.  Last plattest --test ibus-x11 (X11 of
WSLg; skipped without an X server): a raw X11 client (icc_x11.py's) finds its unmapped window
by title and sends it synthetic events -- FocusIn, the keys 'a' and 's' (pressed and released),
FocusOut; the X11 backend must hand both keys to IBus with their evdev code.  Exit code 0 =
passed.
"""
import argparse, os, socket, struct, subprocess, sys, tempfile, threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from icc_x11 import X  # noqa: E402
from portal_bus import R, W, listen_unix, message, parse  # noqa: E402

CTX = "/org/freedesktop/IBus/InputContext_7"
IC = "org.freedesktop.IBus.InputContext"
RELEASE = 1 << 30
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def ibus_text(s):
    """an IBusText variant: (sa{sv}sv) with an empty IBusAttrList"""
    return ("(sa{sv}sv)", ("IBusText", {}, s, ("(sa{sv}av)", ("IBusAttrList", {}, []))))


def serve(path, rec, mode):
    srv = listen_unix(path, 1)
    srv.settimeout(20)
    try:
        c, _ = srv.accept()
    except socket.timeout:
        rec["error"] = "no connection"
        return
    c.settimeout(15)
    buf = b""
    while b"BEGIN\r\n" not in buf:
        d = c.recv(4096)
        if not d:
            return
        buf += d
        if b"AUTH EXTERNAL" in buf and b"\r\n" in buf.split(b"AUTH", 1)[1] and not rec.get("auth"):
            rec["auth"] = True
            c.sendall(b"OK 0123456789abcdef0123456789abcdef\r\n")
    buf = buf.split(b"BEGIN\r\n", 1)[1]
    ser = [100]

    def send(mtype, fields, sig="", body=b""):
        ser[0] += 1
        f = fields + ([(8, "g", sig)] if sig else [])
        c.sendall(message(mtype, ser[0], f, body))

    def reply(m, sig="", body=b""):
        send(2, [(5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", "org.freedesktop.DBus")], sig, body)

    def error(m, name, text):
        send(3, [(4, "s", name), (5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", ":1.5")], "s", W().all("s", [text]))

    def signal(member, sig="", vals=()):
        send(4, [(1, "o", CTX), (2, "s", IC), (3, "s", member), (7, "s", ":1.5")], sig, W().all(sig, list(vals)) if sig else b"")

    while True:
        m, buf = parse(buf)
        if m is None:
            try:
                d = c.recv(65536)
            except (socket.timeout, OSError):
                return
            if not d:
                return
            buf += d
            continue
        fl = m["fields"]
        member, sig = fl.get(3, ""), fl.get(8, "")
        body = R(m["body"]).all(sig) if sig else []
        if m["type"] != 1:
            if m["type"] == 3:
                rec.setdefault("errors", []).append(fl.get(4))
            continue
        rec.setdefault("members", []).append(member)
        if member == "Hello":
            reply(m, "s", W().all("s", [":1.42"]))
        elif member == "AddMatch":
            rec.setdefault("matches", []).append(body[0])
            reply(m)
        elif member == "CreateInputContext":
            rec["create"] = (fl.get(6), fl.get(1), fl.get(2), body)
            if mode == "none":
                error(m, "org.freedesktop.DBus.Error.ServiceUnknown", "no input method")
            else:
                reply(m, "o", W().all("o", [CTX]))
        elif member == "SetCapabilities":
            rec["caps"] = (fl.get(1), body)
            reply(m)
        elif member == "SetCursorLocation":
            rec.setdefault("locations", []).append(tuple(v - (1 << 32) if v >= 1 << 31 else v for v in body))
            reply(m)
        elif member == "ProcessKeyEvent":
            keyval, keycode, state = body
            rec.setdefault("keys", []).append((keyval, keycode, state))
            took = True
            if state & RELEASE:
                took = False
            elif keyval == 0x61:
                signal("UpdatePreeditText", "vub", [ibus_text("かな"), 1, True])
            elif keyval == 0xFF0D:
                signal("CommitText", "v", [ibus_text("かな")])
            elif keyval == 0xFF08:
                signal("ForwardKeyEvent", "uuu", [0xFF51, 105, 0])
                signal("ForwardKeyEvent", "uuu", [0x63, 46, 0])
            elif keyval == 0x62:
                signal("UpdatePreeditText", "vub", [ibus_text("b"), 1, True])
            elif keyval == 0xFF1B:
                signal("HidePreeditText")
            elif keyval == 0x71:
                continue  # no answer
            else:
                took = False
            reply(m, "b", W().all("b", [took]))
        else:
            reply(m)


def run(exe, test, mode, rec):
    with tempfile.TemporaryDirectory() as d:
        sock = os.path.join(d, "bus")
        t = None
        if mode != "unreachable":
            t = threading.Thread(target=serve, args=(listen_unix(sock, 1), rec, mode), daemon=True)
            t.start()
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}")
        env.pop("jfgui_IME", None)
        r = subprocess.run([str(exe), "--test", test], env=env, capture_output=True, text=True, timeout=60)
        if t:
            t.join(5)
    print(r.stdout.strip())
    return r


def x_window(x, title):
    """the top-level window whose _NET_WM_NAME is title (QueryTree of the root)"""
    name = x.atom("_NET_WM_NAME")
    x.req(15, 0, struct.pack("<I", x.root))
    r = x.reply()
    n, = struct.unpack_from("<H", r, 16)
    for k in range(n):
        w, = struct.unpack_from("<I", r, 32 + 4 * k)
        x.req(20, 0, struct.pack("<IIIII", w, name, 0, 0, 64))
        p = x.reply()
        if p[32:32 + struct.unpack_from("<I", p, 16)[0]] == title.encode():
            return w
    return 0


def x_send(x, win, mask, ev):
    """SendEvent(propagate false) of a 32-byte event"""
    x.req(25, 0, struct.pack("<II", win, mask) + ev.ljust(32, b"\0"))
    x.sync()


def x_focus(x, win, fin):
    x_send(x, win, 1 << 21, struct.pack("<BBHIB", 9 if fin else 10, 3, 0, win, 0))


def x_key(x, win, kc, press, time):
    ev = struct.pack("<BBHIIIIhhhhHBx", 2 if press else 3, kc, 0, time, x.root, win, 0, 0, 0, 5, 5, 0, 1)
    x_send(x, win, 1 if press else 2, ev)


def run_x11(exe, rec):
    """plattest --test ibus-x11 against the fake bus and synthetic X events -> its result"""
    with tempfile.TemporaryDirectory() as d:
        sock = os.path.join(d, "bus")
        t = threading.Thread(target=serve, args=(listen_unix(sock, 1), rec, "ok"), daemon=True)
        t.start()
        # jfgui_LIFE=0: the window starts the lifecycle thread, whose own session connection
        # would otherwise take this one-connection bus before the input context's
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}", jfgui_BACKEND="x11", jfgui_LIFE="0")
        env.pop("jfgui_IME", None)
        p = subprocess.Popen([str(exe), "--test", "ibus-x11"], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        out = []

        def until(what):
            while True:
                ln = p.stdout.readline()
                if not ln:
                    return False
                out.append(ln.rstrip("\n"))
                if what in ln:
                    return True

        x = X()
        if until("ibus-x11: window"):
            win = x_window(x, "plattest ibus")
            rec["xwin"] = win
            x_focus(x, win, True)
            if until("ibus-x11: ime on"):
                for kc, press, tm in ((38, True, 1000), (38, False, 1050), (39, True, 1100), (39, False, 1150)):
                    x_key(x, win, kc, press, tm)
                if until("ibus-x11: keys seen"):
                    x_focus(x, win, False)
        until("\0")  # the rest
        err = p.stderr.read()
        code = p.wait(30)
        t.join(5)
    print("\n".join(out))
    return code, err


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"

    rec = {}
    r = run(exe, "ibus-bus", "ok", rec)
    check("plattest --test ibus-bus passed", r.returncode == 0, r.stderr.strip())
    check("SASL EXTERNAL", rec.get("auth"), rec)
    check("CreateInputContext(\"jfgui\") on the portal",
          rec.get("create") == ("org.freedesktop.portal.IBus", "/org/freedesktop/IBus", "org.freedesktop.IBus.Portal", ["jfgui"]),
          rec.get("create"))
    check("a match for the context's signals",
          any(f"path='{CTX}'" in x and f"interface='{IC}'" in x for x in rec.get("matches", [])), rec.get("matches"))
    check("SetCapabilities(preedit | focus)", rec.get("caps") == (CTX, [9]), rec.get("caps"))
    mem = rec.get("members", [])
    check("FocusIn once before the keys, FocusOut at the end",
          mem.count("FocusIn") == 1 and mem.count("FocusOut") == 1
          and "ProcessKeyEvent" in mem and mem.index("FocusIn") < mem.index("ProcessKeyEvent") < mem.index("FocusOut"), mem)
    check("the caret locations on the root", rec.get("locations") == [(100, 200, 1, 18), (104, 200, 1, 18)], rec.get("locations"))
    want = [(0x61, 30, 0), (0x61, 30, RELEASE), (0xFF0D, 28, 0), (0x7A, 44, 0), (0xFF08, 14, 0), (0x62, 48, 0),
            (0xFF1B, 1, 0), (0x71, 16, 0), (0x62, 48, 0)]
    check("ProcessKeyEvent(keyval, keycode, state) in order", rec.get("keys") == want, rec.get("keys"))
    check("no error answers", not rec.get("errors"), rec.get("errors"))

    rec = {}
    r = run(exe, "ibus-none", "none", rec)
    check("plattest --test ibus-none passed (the portal answers with an error)", r.returncode == 0, r.stderr.strip())
    check("it asked once and sent nothing more",
          rec.get("create") is not None and "ProcessKeyEvent" not in rec.get("members", []) and "FocusIn" not in rec.get("members", []),
          rec.get("members"))
    r = run(exe, "ibus-none", "unreachable", {})
    check("plattest --test ibus-none passed (no bus)", r.returncode == 0, r.stderr.strip())

    disp = os.environ.get("DISPLAY", ":0").split(":")[-1].split(".")[0]
    if not os.path.exists(f"/tmp/.X11-unix/X{disp}"):
        print("SKIP ibus-x11: no X server")
    else:
        rec = {}
        code, err = run_x11(exe, rec)
        check("plattest --test ibus-x11 passed", code == 0, err.strip())
        check("the unmapped window was found by its title", rec.get("xwin", 0) != 0, rec)
        mem = rec.get("members", [])
        check("X11: FocusIn once, SetCursorLocation once (the same caret every frame), FocusOut once",
              mem.count("FocusIn") == 1 and mem.count("SetCursorLocation") == 1 and mem.count("FocusOut") == 1, mem)
        loc = rec.get("locations", [])
        check("X11: the caret's height", len(loc) == 1 and loc[0][2:] == (1, 16), loc)
        want = [(0x61, 30, 0), (0x61, 30, RELEASE), (0x73, 31, 0), (0x73, 31, RELEASE)]
        check("X11: keysym, evdev code and state of both keys", rec.get("keys") == want, rec.get("keys"))
    print("ibus_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
