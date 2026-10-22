#!/usr/bin/env python3
"""atspi_bus.py -- the accessibility tree of lib/ui on AT-SPI2 (lib/plat/linux_atspi.rs,
lib/ui/a11y.rs) against a fake session bus and a fake accessibility bus.  Run inside WSL / Linux
from the repository root (needs a display: WSLg):

  python3 tools/platshared/atspi_bus.py [--rs build]          the fake buses (always)
  python3 tools/platshared/atspi_bus.py [--rs build] --real   also against the real at-spi2-core
                                                              (dbus-run-session, at-spi-bus-launcher,
                                                              at-spi2-registryd) through libatspi

The fake buses (the marshalling of portal_bus.py) play the session bus (org.a11y.Bus: IsEnabled,
GetAddress), the accessibility bus daemon, the registry (Socket.Embed) and a screen reader.
plattest --test a11y-bus shows its demo window; the screen reader walks the tree (roles, names,
states, interfaces, parents, extents, the node at a point, Value, Text, a heading with its level
attribute, the localized role names: a second run with --lang en has them in English), clicks "Drücken",
toggles "Option", sets the slider to 75 and the text to "Bert", selects the second tab, follows the
caret of the entry as a screen magnifier does (Focus brings TextCaretMoved along, the caret's
GetCharacterExtents lie in the entry on the screen, SetCaretOffset / SetSelection move it and the
extents), moves the focus and waits for the events of each (PropertyChange, StateChanged,
TextChanged, TextCaretMoved, Focus), then presses "Beenden".  plattest checks the app state at its
end.  Exit code 0 = passed.

--real runs the same steps in a private session bus with at-spi2-core's own bus launcher and
registry; the screen reader is libatspi (libatspi.so.0 through ctypes, the library Orca uses),
which also caches what the events say.
"""
import argparse, ctypes, os, socket, subprocess, sys, tempfile, threading, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, message, parse  # noqa: E402

fails = 0
APP = ":1.42"     # the app's name on both fake buses
AT = ":1.7"       # the fake screen reader
ROOT = "/org/a11y/atspi/accessible/root"
ACC = "org.a11y.atspi.Accessible"
PROPS = "org.freedesktop.DBus.Properties"
ROLE = {"frame": 23, "label": 29, "push button": 43, "check box": 7, "radio button": 44, "toggle button": 62,
        "slider": 51, "progress bar": 42, "entry": 79, "list box": 98, "list item": 32, "page tab list": 38,
        "page tab": 37, "application": 75, "heading": 83}
ST = {"active": 1, "checked": 4, "enabled": 8, "focusable": 11, "focused": 12, "selected": 23, "sensitive": 24,
      "showing": 25, "visible": 30, "checkable": 41, "editable": 7}


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def accept(path):
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(1)
    srv.settimeout(30)
    return srv


def sasl(c, rec):
    buf = b""
    while b"BEGIN\r\n" not in buf:
        d = c.recv(4096)
        if not d:
            return None
        buf += d
        if b"AUTH EXTERNAL" in buf and b"\r\n" in buf.split(b"AUTH", 1)[1] and not rec.get("auth"):
            rec["auth"] = True
            c.sendall(b"OK 0123456789abcdef0123456789abcdef\r\n")
    return buf.split(b"BEGIN\r\n", 1)[1]


def session_bus(path, a11y_addr, rec):
    """the session bus: Hello, AddMatch, org.a11y.Bus (IsEnabled true, GetAddress); every
    connection of the app (the settings portal asks on one of its own) in a thread"""
    srv = accept(path)
    first = True
    while True:
        try:
            c, _ = srv.accept()
        except (socket.timeout, OSError):
            if first:
                rec["error"] = "no session connection"
            return
        first = False
        threading.Thread(target=session_conn, args=(c, a11y_addr, rec), daemon=True).start()


def session_conn(c, a11y_addr, rec):
    c.settimeout(20)
    buf = sasl(c, {})
    rec["auth"] = True
    ser = [100]

    def reply(m, sig="", body=b""):
        ser[0] += 1
        f = [(5, "u", m["serial"]), (6, "s", APP), (7, "s", "org.freedesktop.DBus")] + ([(8, "g", sig)] if sig else [])
        c.sendall(message(2, ser[0], f, body))

    while buf is not None:
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
            continue
        rec.setdefault("session", []).append(member)
        if member == "Hello":
            reply(m, "s", W().all("s", [APP]))
        elif member == "Get":
            rec.setdefault("asked", []).append(body[1])
            reply(m, "v", W().all("v", [("b", 1)]))
        elif member == "GetAddress":
            reply(m, "s", W().all("s", [a11y_addr]))
        else:
            reply(m)


class A11yBus:
    """the accessibility bus with the registry and a screen reader (calls from the main thread)"""

    def __init__(self, path):
        self.srv = accept(path)
        self.c = None
        self.ser = 1000
        self.lock = threading.Condition()
        self.replies = {}
        self.signals = []
        self.rec = {}
        self.embedded = None
        threading.Thread(target=self.loop, daemon=True).start()

    def send(self, mtype, fields, sig="", body=b""):
        with self.lock:
            self.ser += 1
            s = self.ser
        f = fields + ([(8, "g", sig)] if sig else [])
        self.c.sendall(message(mtype, s, f, body))
        return s

    def loop(self):
        try:
            self.c, _ = self.srv.accept()
        except socket.timeout:
            self.rec["error"] = "no accessibility bus connection"
            return
        self.c.settimeout(None)
        buf = sasl(self.c, self.rec)
        while buf is not None:
            m, buf = parse(buf)
            if m is None:
                try:
                    d = self.c.recv(65536)
                except OSError:
                    return
                if not d:
                    with self.lock:
                        self.rec["closed"] = True
                        self.lock.notify_all()
                    return
                buf += d
                continue
            fl = m["fields"]
            member, sig = fl.get(3, ""), fl.get(8, "")
            body = R(m["body"]).all(sig) if sig else []
            if m["type"] == 1:  # calls of the app: the bus daemon and the registry
                def reply(rsig="", rbody=b"", sender="org.freedesktop.DBus"):
                    self.send(2, [(5, "u", m["serial"]), (6, "s", APP), (7, "s", sender)], rsig, rbody)
                if member == "Hello":
                    reply("s", W().all("s", [APP]))
                elif member == "Embed":
                    with self.lock:
                        self.embedded = (fl.get(1), fl.get(2), fl.get(6), body)
                        self.lock.notify_all()
                    reply("(so)", W().all("(so)", [(":1.1", ROOT)]), ":1.1")
                else:
                    reply()
            elif m["type"] in (2, 3):
                with self.lock:
                    self.replies[fl.get(5)] = (m["type"], body, fl.get(4))
                    self.lock.notify_all()
            elif m["type"] == 4:
                with self.lock:
                    self.signals.append((fl.get(1), fl.get(2), member, body))
                    self.lock.notify_all()

    def wait(self, pred, timeout=10):
        end = time.time() + timeout
        with self.lock:
            while not pred():
                left = end - time.time()
                if left <= 0:
                    return False
                self.lock.wait(left)
            return True

    def call(self, path, iface, member, sig="", args=(), timeout=5):
        """-> body list; raises RuntimeError on an error answer"""
        s = self.send(1, [(1, "o", path), (2, "s", iface), (3, "s", member), (6, "s", APP), (7, "s", AT)],
                      sig, W().all(sig, list(args)) if sig else b"")
        if not self.wait(lambda: s in self.replies, timeout):
            raise TimeoutError(f"{member} on {path}")
        with self.lock:
            typ, body, err = self.replies.pop(s)
        if typ == 3:
            raise RuntimeError(err)
        return body

    def prop(self, path, iface, name):
        return self.call(path, PROPS, "Get", "ss", [iface, name])[0][1]

    def signal(self, member, path=None, detail=None, since=0, timeout=8):
        """-> the body of a signal (after index since) or None"""
        def find():
            for p, _, mem, body in self.signals[since:]:
                if mem == member and (path is None or p == path) and (detail is None or body and body[0] == detail):
                    return body
            return None
        self.wait(lambda: find() is not None, timeout)
        with self.lock:
            return find()


def states(bits):
    v = bits[0] | bits[1] << 32
    return {n for n, b in ST.items() if v >> b & 1}


def walk(bus, path, depth=0, out=None):
    """every object below path: dicts with path, role, name, states, interfaces, kids, parent"""
    out = [] if out is None else out
    kids = bus.call(path, ACC, "GetChildren")[0]
    for i, (_, p) in enumerate(kids):
        o = {"path": p, "role": bus.call(p, ACC, "GetRole")[0], "name": bus.prop(p, ACC, "Name"),
             "states": states(bus.call(p, ACC, "GetState")[0]), "ifaces": bus.call(p, ACC, "GetInterfaces")[0],
             "parent": bus.prop(p, ACC, "Parent")[1], "index": bus.call(p, ACC, "GetIndexInParent")[0],
             "count": bus.prop(p, ACC, "ChildCount"), "depth": depth, "at": i, "up": path}
        out.append(o)
        walk(bus, p, depth + 1, out)
    return out


def fake(exe, lang=None):
    with tempfile.TemporaryDirectory() as d:
        sess, a11y = os.path.join(d, "session"), os.path.join(d, "a11y")
        srec = {}
        threading.Thread(target=session_bus, args=(sess, f"unix:path={a11y},guid=0123", srec), daemon=True).start()
        bus = A11yBus(a11y)
        env = {k: v for k, v in os.environ.items() if k not in ("jfgui_A11Y", "NO_AT_BRIDGE")}
        env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={sess}"
        p = subprocess.Popen([str(exe), "--test", "a11y-bus", "--auto-close", "60000"] + (["--lang", lang] if lang else []),
                             env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            screen_reader(bus, srec, lang or "de")
        except Exception as e:  # noqa: BLE001
            check("the screen reader ran through", False, repr(e))
            p.kill()
        try:
            out, err = p.communicate(timeout=20)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
    print(out.strip())
    check(f"plattest --test a11y-bus passed ({lang or 'de'})", p.returncode == 0, (p.returncode, err.strip()[-800:]))


# GetLocalizedRoleName of the heading, a push button, the switch, the window and the application
# in the UI language (plattest --lang); GetRoleName stays English
LOCALIZED = {"de": ("Überschrift", "Schaltfläche", "Umschalter", "Rahmen", "Anwendung"),
             "en": ("heading", "push button", "toggle switch", "frame", "application")}


def screen_reader(bus, srec, lang="de"):
    check("embedded in the registry", bus.wait(lambda: bus.embedded is not None, 20), srec)
    if not bus.embedded:
        return
    path, iface, dest, body = bus.embedded
    check("Socket.Embed with the root reference", (path, iface, dest) == (ROOT, "org.a11y.atspi.Socket", "org.a11y.atspi.Registry")
          and body == [(APP, ROOT)], bus.embedded)
    check("asked org.a11y.Status first", "IsEnabled" in srec.get("asked", []), srec)
    check("application root", bus.call(ROOT, ACC, "GetRole")[0] == ROLE["application"]
          and bus.prop(ROOT, ACC, "Name") == "a11y-Test" and bus.prop(ROOT, "org.a11y.atspi.Application", "ToolkitName") == "JFLib")
    check("the root's parent is the registry", bus.prop(ROOT, ACC, "Parent") == (":1.1", ROOT))
    bus.call(ROOT, PROPS, "Set", "ssv", ["org.a11y.atspi.Application", "Id", ("i", 5)])
    check("the registry sets the application id", bus.prop(ROOT, "org.a11y.atspi.Application", "Id") == 5)
    # the first tree arrives with the next frame
    wins = []
    end = time.time() + 15
    while time.time() < end and not wins:
        wins = bus.call(ROOT, ACC, "GetChildren")[0]
        time.sleep(0.1)
    check("one window", len(wins) == 1, wins)
    if not wins:
        return
    w = wins[0][1]
    check("the window: frame, its title", bus.call(w, ACC, "GetRole")[0] == ROLE["frame"] and bus.prop(w, ACC, "Name") == "a11y-Test")
    tree = walk(bus, w)

    def find(role, name):
        for o in tree:
            if o["role"] == ROLE[role] and o["name"] == name:
                return o
        return None
    lab, btn, off = find("label", "Klicks: 0"), find("push button", "Drücken"), find("push button", "Gesperrt")
    chk, sld, txt = find("check box", "Option"), find("slider", "Lautstärke"), find("entry", "Name")
    tl = next((o for o in tree if o["role"] == ROLE["page tab list"]), None)
    lb = next((o for o in tree if o["role"] == ROLE["list box"]), None)
    prg = next((o for o in tree if o["role"] == ROLE["progress bar"]), None)
    quit_ = find("push button", "Beenden")
    check("label, buttons, check box, slider, entry, tab strip, list, progress bar",
          all([lab, btn, off, chk, sld, txt, tl, lb, prg, quit_]), [(o["role"], o["name"]) for o in tree])
    if not all([lab, btn, off, chk, sld, txt, tl, lb, prg, quit_]):
        return
    check("button: enabled, focusable, Action", {"enabled", "sensitive", "focusable", "showing"} <= btn["states"]
          and "org.a11y.atspi.Action" in btn["ifaces"] and "org.a11y.atspi.Component" in btn["ifaces"], btn)
    check("disabled button: not enabled, no Action", "enabled" not in off["states"] and "org.a11y.atspi.Action" not in off["ifaces"], off)
    check("check box: checkable, not checked", "checkable" in chk["states"] and "checked" not in chk["states"], chk)
    tabs = [o for o in tree if o["up"] == tl["path"]]
    check("tab strip: two page tabs, the first selected", [o["name"] for o in tabs] == ["Erste", "Zweite"]
          and all(o["role"] == ROLE["page tab"] for o in tabs) and "selected" in tabs[0]["states"], tabs)
    rows = [o for o in tree if o["up"] == lb["path"]]
    check("list box: three list items", [o["name"] for o in rows] == ["Apfel", "Birne", "Kirsche"]
          and all(o["role"] == ROLE["list item"] for o in rows), rows)
    check("parents and indices agree with the children", all(o["parent"] == o["up"] and o["index"] == o["at"] for o in tree)
          and all(o["count"] == sum(1 for k in tree if k["up"] == o["path"]) for o in tree))
    vi = "org.a11y.atspi.Value"
    check("slider: Value 30 in 0 ..= 100", (bus.prop(sld["path"], vi, "CurrentValue"), bus.prop(sld["path"], vi, "MinimumValue"),
                                            bus.prop(sld["path"], vi, "MaximumValue")) == (30.0, 0.0, 100.0))
    check("progress bar: Value 40", bus.prop(prg["path"], vi, "CurrentValue") == 40.0)
    ti = "org.a11y.atspi.Text"
    check("entry: Text 'Anna', EditableText", bus.call(txt["path"], ti, "GetText", "ii", [0, -1])[0] == "Anna"
          and bus.prop(txt["path"], ti, "CharacterCount") == 4 and "org.a11y.atspi.EditableText" in txt["ifaces"]
          and "editable" in txt["states"])
    check("label: Text", bus.call(lab["path"], ti, "GetText", "ii", [0, 6])[0] == "Klicks"
          and bus.call(lab["path"], ti, "GetStringAtOffset", "iu", [8, 1]) == ["0", 8, 9])
    # a heading (ui.heading level 2): role heading, the attribute level, its text; no control
    head = find("heading", "Einstellungen")
    check("heading: role heading, GetRoleName 'heading'", head is not None
          and bus.call(head["path"], ACC, "GetRoleName")[0] == "heading", [(o["role"], o["name"]) for o in tree])
    if head:
        check("heading: GetAttributes level 2", bus.call(head["path"], ACC, "GetAttributes")[0] == {"level": "2"},
              bus.call(head["path"], ACC, "GetAttributes"))
        check("heading: Text 'Einstellungen', no Action, not focusable", ti in head["ifaces"]
              and bus.call(head["path"], ti, "GetText", "ii", [0, -1])[0] == "Einstellungen"
              and "org.a11y.atspi.Action" not in head["ifaces"] and "focusable" not in head["states"], head)
    check("label: no attributes", bus.call(lab["path"], ACC, "GetAttributes")[0] == {})
    sw = find("toggle button", "Schalter")
    loc = lambda path: bus.call(path, ACC, "GetLocalizedRoleName")[0]  # noqa: E731
    got = (loc(head["path"]) if head else None, loc(btn["path"]), loc(sw["path"]) if sw else None, loc(w), loc(ROOT))
    check(f"GetLocalizedRoleName in the UI language ({lang})", got == LOCALIZED[lang], got)
    check("GetRoleName stays English", bus.call(btn["path"], ACC, "GetRoleName")[0] == "push button"
          and (sw is None or bus.call(sw["path"], ACC, "GetRoleName")[0] == "toggle button"))
    ext = bus.call(btn["path"], "org.a11y.atspi.Component", "GetExtents", "u", [1])[0]
    check("button extents (window)", ext[2] > 0 and ext[3] > 0, ext)
    at = bus.call(w, "org.a11y.atspi.Component", "GetAccessibleAtPoint", "iiu", [ext[0] + ext[2] // 2, ext[1] + ext[3] // 2, 1])[0]
    check("the node at the button's centre is the button", at == (APP, btn["path"]), at)
    try:
        bus.call("/org/a11y/atspi/accessible/n0_ffffffff", ACC, "GetRole")
        check("an unknown object answers with an error", False)
    except RuntimeError as e:
        check("an unknown object answers with an error", "UnknownObject" in str(e), e)

    # actions and their events
    n = len(bus.signals)
    check("button: one action 'click'", bus.call(btn["path"], "org.a11y.atspi.Action", "GetActions")[0] == [("click", "click", "")])
    check("DoAction(0) on the button", bus.call(btn["path"], "org.a11y.atspi.Action", "DoAction", "i", [0])[0] == 1)
    ev = bus.signal("PropertyChange", lab["path"], "accessible-name", n)
    check("PropertyChange accessible-name 'Klicks: 1'", ev is not None and ev[3] == ("s", "Klicks: 1"), ev)
    n = len(bus.signals)
    bus.call(chk["path"], "org.a11y.atspi.Action", "DoAction", "i", [0])
    ev = bus.signal("StateChanged", chk["path"], "checked", n)
    check("check box: StateChanged checked 1", ev is not None and ev[1] == 1, ev)
    check("check box: checked now", "checked" in states(bus.call(chk["path"], ACC, "GetState")[0]))
    n = len(bus.signals)
    bus.call(sld["path"], PROPS, "Set", "ssv", [vi, "CurrentValue", ("d", 75.0)])
    ev = bus.signal("PropertyChange", sld["path"], "accessible-value", n)
    check("slider: PropertyChange accessible-value, 75", ev is not None and bus.prop(sld["path"], vi, "CurrentValue") == 75.0, ev)
    n = len(bus.signals)
    ok = bus.call(txt["path"], "org.a11y.atspi.EditableText", "SetTextContents", "s", ["Bert"])[0]
    ev = bus.signal("TextChanged", txt["path"], "insert", n)
    check("entry: SetTextContents, TextChanged insert", ok == 1 and ev is not None
          and bus.call(txt["path"], ti, "GetText", "ii", [0, -1])[0] == "Bert", ev)
    n = len(bus.signals)
    bus.call(tabs[1]["path"], "org.a11y.atspi.Action", "DoAction", "i", [0])
    ev = bus.signal("StateChanged", tabs[1]["path"], "selected", n)
    check("tab: StateChanged selected 1", ev is not None and ev[1] == 1, ev)
    caret(bus, txt, ti)
    n = len(bus.signals)
    check("GrabFocus on the check box", bus.call(chk["path"], "org.a11y.atspi.Component", "GrabFocus")[0] == 1)
    ev = bus.signal("StateChanged", chk["path"], "focused", n)
    fo = bus.signal("Focus", chk["path"], None, n)
    check("focus: StateChanged focused 1 and Focus", ev is not None and ev[1] == 1 and fo is not None, (ev, fo))
    check("the window was announced", any(s[2] in ("Create", "Activate") or (s[2] == "ChildrenChanged" and s[0] == ROOT)
                                          for s in bus.signals), [s[2] for s in bus.signals[:8]])
    bus.call(quit_["path"], "org.a11y.atspi.Action", "DoAction", "i", [0])


def caret(bus, txt, ti):
    """(ROADMAP 11) the entry's caret as GNOME Shell's zoom / Orca follow it: the focus brings
    TextCaretMoved along, the caret's character extents (screen) lie in the entry, a caret or a
    selection set through Text moves them"""
    p = txt["path"]
    comp = "org.a11y.atspi.Component"
    n = len(bus.signals)
    check("GrabFocus on the entry", bus.call(p, comp, "GrabFocus")[0] == 1)
    fo = bus.signal("Focus", p, None, n)
    cm = bus.signal("TextCaretMoved", p, None, n)
    check("entry: Focus, then TextCaretMoved with the caret after 'Bert'", fo is not None and cm is not None and cm[1] == 4, (fo, cm))
    check("entry: CaretOffset 4", bus.prop(p, ti, "CaretOffset") == 4)
    ex = bus.call(p, comp, "GetExtents", "u", [0])[0]
    ce = bus.call(p, ti, "GetCharacterExtents", "iu", [4, 0])
    check("the caret's character extents lie in the entry (screen coordinates)",
          ex[0] < ce[0] < ex[0] + ex[2] and ex[1] <= ce[1] and ce[1] + ce[3] <= ex[1] + ex[3] and ce[3] > 0, (ce, ex))
    cw = bus.call(p, ti, "GetCharacterExtents", "iu", [4, 1])
    check("... and in window coordinates", (ce[0] - cw[0], ce[1] - cw[1]) == (ex[0] - bus.call(p, comp, "GetExtents", "u", [1])[0][0],
                                                                         ex[1] - bus.call(p, comp, "GetExtents", "u", [1])[0][1]), (cw, ce))
    n = len(bus.signals)
    check("SetCaretOffset(1)", bus.call(p, ti, "SetCaretOffset", "i", [1])[0] == 1)
    cm = bus.signal("TextCaretMoved", p, None, n)
    check("TextCaretMoved 1, CaretOffset 1", cm is not None and cm[1] == 1 and bus.prop(p, ti, "CaretOffset") == 1, cm)
    c1 = bus.call(p, ti, "GetCharacterExtents", "iu", [1, 0])
    check("the caret's extents moved to the left", c1[0] < ce[0] and c1[3] == ce[3], (c1, ce))
    check("GetOffsetAtPoint at the caret: 1", bus.call(p, ti, "GetOffsetAtPoint", "iiu", [c1[0] + 1, c1[1] + 1, 0])[0] == 1)
    n = len(bus.signals)
    bus.call(p, ti, "SetSelection", "iii", [0, 1, 3])
    cm = bus.signal("TextCaretMoved", p, None, n)
    check("SetSelection(1, 3): TextCaretMoved 3, GetSelection 1 .. 3", cm is not None and cm[1] == 3
          and bus.call(p, ti, "GetNSelections")[0] == 1 and bus.call(p, ti, "GetSelection", "i", [0]) == [1, 3], cm)


# ------------------------------------------------------------------ the real at-spi2-core (libatspi)
def real(exe, rs):
    """re-run inside a private session bus"""
    env = dict(os.environ, jfgui_A11Y_INNER="1")
    r = subprocess.run(["dbus-run-session", "--", sys.executable, __file__, "--rs", rs, "--real-inner"], env=env, timeout=180)
    check("real at-spi2-core: all passed", r.returncode == 0, r.returncode)


def real_inner(exe):
    at = ctypes.CDLL("libatspi.so.0")
    go = ctypes.CDLL("libgobject-2.0.so.0")
    gl = ctypes.CDLL("libglib-2.0.so.0")
    vp, cp = ctypes.c_void_p, ctypes.c_char_p
    for name, res, args in [
        ("atspi_init", ctypes.c_int, []), ("atspi_get_desktop", vp, [ctypes.c_int]),
        ("atspi_accessible_get_child_count", ctypes.c_int, [vp, vp]),
        ("atspi_accessible_get_child_at_index", vp, [vp, ctypes.c_int, vp]),
        ("atspi_accessible_get_name", vp, [vp, vp]), ("atspi_accessible_get_role", ctypes.c_int, [vp, vp]),
        ("atspi_accessible_get_toolkit_name", vp, [vp, vp]),
        ("atspi_accessible_get_state_set", vp, [vp]), ("atspi_state_set_contains", ctypes.c_int, [vp, ctypes.c_int]),
        ("atspi_accessible_get_action_iface", vp, [vp]), ("atspi_action_do_action", ctypes.c_int, [vp, ctypes.c_int, vp]),
        ("atspi_accessible_get_value_iface", vp, [vp]), ("atspi_value_get_current_value", ctypes.c_double, [vp, vp]),
        ("atspi_value_set_current_value", ctypes.c_int, [vp, ctypes.c_double, vp]),
        ("atspi_accessible_get_text_iface", vp, [vp]), ("atspi_text_get_text", vp, [vp, ctypes.c_int, ctypes.c_int, vp]),
        ("atspi_accessible_get_editable_text_iface", vp, [vp]),
        ("atspi_editable_text_set_text_contents", ctypes.c_int, [vp, cp, vp]),
        ("atspi_accessible_get_parent", vp, [vp, vp]),
        ("atspi_text_get_caret_offset", ctypes.c_int, [vp, vp]), ("atspi_text_set_caret_offset", ctypes.c_int, [vp, ctypes.c_int, vp]),
        ("atspi_text_get_character_extents", vp, [vp, ctypes.c_int, ctypes.c_int, vp]),
        ("atspi_component_grab_focus", ctypes.c_int, [vp, vp]), ("atspi_accessible_get_component_iface", vp, [vp]),
        ("atspi_accessible_clear_cache", None, [vp]),
        ("atspi_accessible_get_attributes_as_array", vp, [vp, vp]),
    ]:
        f = getattr(at, name)
        f.restype, f.argtypes = res, args
    gl.g_free.argtypes = [vp]
    go.g_object_unref.argtypes = [vp]
    gl.g_main_context_iteration.argtypes = [vp, ctypes.c_int]

    def s(p):
        if not p:
            return None
        v = ctypes.string_at(p).decode()
        gl.g_free(p)
        return v

    def spin(sec):
        end = time.time() + sec
        while time.time() < end:
            gl.g_main_context_iteration(None, 0)
            time.sleep(0.01)

    check("libatspi: atspi_init", at.atspi_init() in (0, 1))
    env = dict(os.environ, jfgui_A11Y="1")
    p = subprocess.Popen([str(exe), "--test", "a11y-bus", "--auto-close", "60000"], env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        desk = at.atspi_get_desktop(0)
        app = None
        end = time.time() + 20
        while time.time() < end and not app:
            for i in range(at.atspi_accessible_get_child_count(desk, None)):
                a = at.atspi_accessible_get_child_at_index(desk, i, None)
                if a and s(at.atspi_accessible_get_name(a, None)) == "a11y-Test":
                    app = a
            spin(0.2)
        check("the registry lists the application", app is not None)
        if not app:
            return
        check("toolkit name", s(at.atspi_accessible_get_toolkit_name(app, None)) == "JFLib")
        win = None
        end = time.time() + 10
        while time.time() < end and not win:
            at.atspi_accessible_clear_cache(app)
            if at.atspi_accessible_get_child_count(app, None) > 0:
                win = at.atspi_accessible_get_child_at_index(app, 0, None)
            spin(0.2)
        check("one window", win is not None and at.atspi_accessible_get_role(win, None) == ROLE["frame"])
        nodes = {}

        def walk_(o, depth=0):
            for i in range(at.atspi_accessible_get_child_count(o, None)):
                k = at.atspi_accessible_get_child_at_index(o, i, None)
                nodes[(at.atspi_accessible_get_role(k, None), s(at.atspi_accessible_get_name(k, None)))] = k
                walk_(k, depth + 1)
        walk_(win)
        btn, chk = nodes.get((ROLE["push button"], "Drücken")), nodes.get((ROLE["check box"], "Option"))
        sld, txt = nodes.get((ROLE["slider"], "Lautstärke")), nodes.get((ROLE["entry"], "Name"))
        quit_, lab = nodes.get((ROLE["push button"], "Beenden")), nodes.get((ROLE["label"], "Klicks: 0"))
        check("libatspi sees the button, check box, slider, entry, label", all([btn, chk, sld, txt, quit_, lab]),
              sorted((r, n) for r, n in nodes))
        if not all([btn, chk, sld, txt, quit_, lab]):
            return
        # the heading as Orca reads it: role heading, the object attribute level:2
        head = nodes.get((ROLE["heading"], "Einstellungen"))
        attrs = []
        if head:
            class GArray(ctypes.Structure):
                _fields_ = [("data", ctypes.POINTER(ctypes.c_char_p)), ("len", ctypes.c_uint)]
            arr = at.atspi_accessible_get_attributes_as_array(head, None)
            if arr:
                ga = ctypes.cast(arr, ctypes.POINTER(GArray)).contents
                attrs = [ga.data[i].decode() for i in range(ga.len)]
        check("libatspi: the heading (role heading) with the attribute level:2", head is not None and "level:2" in attrs, attrs)
        check("the button's parent is the window", at.atspi_accessible_get_parent(btn, None) == win
              or s(at.atspi_accessible_get_name(at.atspi_accessible_get_parent(btn, None), None)) == "a11y-Test")
        check("libatspi: do_action on the button", at.atspi_action_do_action(at.atspi_accessible_get_action_iface(btn), 0, None) == 1)
        spin(1.0)
        check("libatspi: the label's cached name follows the event",
              s(at.atspi_accessible_get_name(lab, None)) == "Klicks: 1")
        at.atspi_action_do_action(at.atspi_accessible_get_action_iface(chk), 0, None)
        spin(1.0)
        check("libatspi: the check box is checked (cached state set)",
              at.atspi_state_set_contains(at.atspi_accessible_get_state_set(chk), ST["checked"]) == 1)
        v = at.atspi_accessible_get_value_iface(sld)
        check("libatspi: slider value 30", at.atspi_value_get_current_value(v, None) == 30.0)
        at.atspi_value_set_current_value(v, 75.0, None)
        spin(1.0)
        check("libatspi: slider value 75", at.atspi_value_get_current_value(v, None) == 75.0)
        e = at.atspi_accessible_get_editable_text_iface(txt)
        at.atspi_editable_text_set_text_contents(e, "Bert".encode(), None)
        spin(1.0)
        check("libatspi: entry text 'Bert'", s(at.atspi_text_get_text(at.atspi_accessible_get_text_iface(txt), 0, -1, None)) == "Bert")
        # the caret as GNOME Shell's zoom reads it: offset and character extents on the screen
        ti_ = at.atspi_accessible_get_text_iface(txt)
        at.atspi_component_grab_focus(at.atspi_accessible_get_component_iface(txt), None)
        spin(1.0)
        check("libatspi: caret offset 4 in the focused entry", at.atspi_text_get_caret_offset(ti_, None) == 4)
        rect = (ctypes.c_int * 4).from_address(at.atspi_text_get_character_extents(ti_, 4, 0, None))
        x4 = rect[0]
        at.atspi_text_set_caret_offset(ti_, 2, None)
        spin(1.0)
        check("libatspi: caret offset 2 (set_caret_offset)", at.atspi_text_get_caret_offset(ti_, None) == 2)
        rect = (ctypes.c_int * 4).from_address(at.atspi_text_get_character_extents(ti_, 2, 0, None))
        check("libatspi: the caret's extents moved to the left", rect[0] < x4 and rect[3] > 0, (list(rect), x4))
        tab = nodes.get((ROLE["page tab"], "Zweite"))
        if tab:
            at.atspi_action_do_action(at.atspi_accessible_get_action_iface(tab), 0, None)
            spin(0.5)
        at.atspi_action_do_action(at.atspi_accessible_get_action_iface(quit_), 0, None)
    finally:
        try:
            out, err = p.communicate(timeout=20)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
        print(out.strip())
        check("plattest --test a11y-bus passed (real)", p.returncode == 0, (p.returncode, err.strip()[-800:]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--real-inner", action="store_true")
    a = ap.parse_args()
    exe = Path(a.rs).resolve() / "linux" / "plattest"
    if a.real_inner:
        real_inner(exe)
    else:
        fake(exe)
        fake(exe, "en")
        if a.real:
            real(exe, a.rs)
    print("atspi_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
