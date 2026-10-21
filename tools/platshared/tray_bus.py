#!/usr/bin/env python3
"""tray_bus.py -- plat's notification-area symbol and notifications (lib/plat/linux_tray.rs,
linux_dbus.rs) against a fake session bus.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/tray_bus.py [--rs build]

The fake bus (SASL EXTERNAL, little-endian messages; the marshalling of portal_bus.py) plays the
bus daemon, org.kde.StatusNotifierWatcher, a panel that shows the item and
org.freedesktop.Notifications.  plattest --test tray-bus sets a symbol with a menu and sends a
notification; the fake then reads the item's properties (GetAll), the menu layout (GetLayout),
activates the symbol, clicks the menu row "Beenden" (Event clicked) and clicks the
notification (ActionInvoked, then NotificationClosed).  plattest must see NoteClicked, Activate
and Menu(2); the fake checks what it was told.  Exit code 0 = passed.
"""
import argparse, os, socket, subprocess, sys, tempfile, threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, message, parse  # noqa: E402

fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def serve(path, rec):
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(1)
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
    calls = {}  # our serial -> what we asked

    def send(mtype, fields, sig="", body=b""):
        ser[0] += 1
        f = fields + ([(8, "g", sig)] if sig else [])
        c.sendall(message(mtype, ser[0], f, body))
        return ser[0]

    def reply(m, sig="", body=b""):
        send(2, [(5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", "org.freedesktop.DBus")], sig, body)

    def ask(path, iface, member, sig="", body=b"", what=None):
        s = send(1, [(1, "o", path), (2, "s", iface), (3, "s", member), (6, "s", ":1.42"), (7, "s", ":1.7")], sig, body)
        calls[s] = what or member

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
        if m["type"] == 1:
            rec.setdefault("members", []).append(member)
            if member == "Hello":
                reply(m, "s", W().all("s", [":1.42"]))
            elif member in ("AddMatch",):
                rec.setdefault("matches", []).append(body[0])
                reply(m)
            elif member in ("RequestName", "ReleaseName"):
                rec[member] = body[0]
                reply(m, "u", W().all("u", [1]))
            elif member == "RegisterStatusNotifierItem":
                rec["registered"] = body[0]
                reply(m)
                # the panel looks at the item
                ask("/StatusNotifierItem", "org.freedesktop.DBus.Properties", "GetAll", "s",
                    W().all("s", ["org.kde.StatusNotifierItem"]))
                ask("/MenuBar", "com.canonical.dbusmenu", "GetLayout", "iias", W().all("iias", [0, -1, []]))
            elif member == "Notify":
                rec["notify"] = body
                reply(m, "u", W().all("u", [77]))
                for mem, s, vals in (("ActionInvoked", "us", [77, "default"]), ("NotificationClosed", "uu", [77, 2])):
                    send(4, [(1, "o", "/org/freedesktop/Notifications"), (2, "s", "org.freedesktop.Notifications"),
                             (3, "s", mem), (7, "s", ":1.3")], s, W().all(s, vals))
            else:
                reply(m)
        elif m["type"] in (2, 3):
            what = calls.pop(fl.get(5, 0), None)
            if m["type"] == 3:
                rec.setdefault("errors", []).append((what, fl.get(4)))
                continue
            if what == "GetAll":
                rec["props"] = body[0]
                ask("/StatusNotifierItem", "org.kde.StatusNotifierItem", "Activate", "ii", W().all("ii", [10, 20]))
            elif what == "GetLayout":
                rec["layout"] = body
                quit_id = None
                for kid in body[1][2]:
                    k = kid[1]
                    if k[1].get("label", ("s", ""))[1] == "Beenden":
                        quit_id = k[0]
                rec["quit_id"] = quit_id
                if quit_id is not None:
                    ask("/MenuBar", "com.canonical.dbusmenu", "Event", "isvu", W().all("isvu", [quit_id, "clicked", ("i", 0), 0]))
        elif m["type"] == 4:
            rec.setdefault("signals", []).append(member)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"
    rec = {}
    with tempfile.TemporaryDirectory() as d:
        sock = os.path.join(d, "bus")
        t = threading.Thread(target=serve, args=(sock, rec), daemon=True)
        t.start()
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}")
        r = subprocess.run([str(exe), "--test", "tray-bus"], env=env, capture_output=True, text=True, timeout=60)
        t.join(5)
    print(r.stdout.strip())
    check("plattest --test tray-bus passed", r.returncode == 0, r.stderr.strip())
    check("SASL EXTERNAL", rec.get("auth"), rec)
    check("matches for notifications and the watcher",
          any("org.freedesktop.Notifications" in x for x in rec.get("matches", []))
          and any("StatusNotifierWatcher" in x for x in rec.get("matches", [])), rec.get("matches"))
    name = rec.get("RequestName", "")
    check("item name org.kde.StatusNotifierItem-<pid>-1", name.startswith("org.kde.StatusNotifierItem-") and name.endswith("-1"), name)
    check("registered with the watcher", rec.get("registered") == name, rec.get("registered"))
    p = rec.get("props", {})
    check("ToolTip, Menu, IconPixmap, Status",
          p.get("ToolTip", ("", (0, 0, 0, 0)))[1][2] == "plattest" and p.get("Menu", ("", ""))[1] == "/MenuBar"
          and p.get("IconPixmap", ("", []))[1] and p.get("IconPixmap")[1][0][:2] == (16, 16)
          and bytes(p.get("IconPixmap")[1][0][2][:4]) == bytes([0xFF, 0x1E, 0x78, 0xE6]) and p.get("Status", ("", ""))[1] == "Active", p)
    lay = rec.get("layout")
    labels = []
    if lay:
        for kid in lay[1][2]:
            labels.append(kid[1][1].get("label", ("s", "-"))[1] if "label" in kid[1][1] else kid[1][1].get("type", ("s", "?"))[1])
    check("menu layout: mnemonic as _, separator, submenu", labels == ["_Öffnen", "separator", "Mehr", "Beenden"], labels)
    sub = lay[1][2][2][1][2] if lay else []
    check("submenu row with a check mark and a doubled _",
          bool(sub) and sub[0][1][1].get("label", ("", ""))[1] == "Sub__eins" and sub[0][1][1].get("toggle-state", ("", 0))[1] == 1, sub)
    n = rec.get("notify", [])
    hints = n[6] if len(n) > 6 else {}
    check("Notify: app, title, body, default action, image-data",
          len(n) == 8 and n[0] == "plattest" and n[3] == "Hallo" and n[4] == "vom Test" and "default" in n[5]
          and hints.get("image-data", ("", (0,)))[1][:6] == (2, 1, 8, 1, 8, 4)
          and bytes(hints["image-data"][1][6]) == bytes([0, 0, 0xFF, 0xFF, 0, 0xFF, 0, 0x80]), n)
    check("no error answers", not rec.get("errors"), rec.get("errors"))
    check("the name is given back", rec.get("ReleaseName") == name, rec.get("ReleaseName"))
    print("tray_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
