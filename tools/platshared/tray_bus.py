#!/usr/bin/env python3
"""tray_bus.py -- plat's notification-area symbol and notifications (lib/plat/linux_tray.rs,
linux_dbus.rs) against a fake session bus.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/tray_bus.py [--rs build]

The fake bus (SASL EXTERNAL, little-endian messages; the marshalling of portal_bus.py) plays the
bus daemon, org.kde.StatusNotifierWatcher, a panel that shows the item and
org.freedesktop.Notifications.  plattest --test tray-bus sets a symbol with a menu and sends a
notification; the fake then reads the item's properties (GetAll), the menu layout (GetLayout),
activates the symbol, clicks the menu row "Beenden" (Event clicked) and clicks the
notification (a NotificationClosed from another client, which must not count, then the
server's ActionInvoked and NotificationClosed).  plattest must see NoteClicked, Activate
and Menu(2); the fake checks what it was told.  Exit code 0 = passed.

Who may act on the item, twice: "kde" -- the watcher's owner (:1.5, kded) is not the panel, the
panel (:1.7, plasmashell) owns org.kde.StatusNotifierHost-77 -- and "gnome" -- the panel owns the
watcher itself (gnome-shell with the AppIndicator extension).  Before the panel, another client
(:1.66) sends a NameOwnerChanged of its own naming itself the watcher, then SecondaryActivate,
a click on the row "Öffnen" and an activation token; it answers plattest's ListNames with a
forged reply naming an org.kde.StatusNotifierHost-evil.  None of it may count: plattest must
not see SecondaryActivate or Menu(1), and must not ask who owns the forged host name.
"""
import argparse, os, socket, subprocess, sys, tempfile, threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, listen_unix, message, parse  # noqa: E402

fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


WATCHER = "org.kde.StatusNotifierWatcher"
HOST = "org.kde.StatusNotifierHost-77"
EVIL_HOST = "org.kde.StatusNotifierHost-evil"
PANEL, KDED, EVIL = ":1.7", ":1.5", ":1.66"


def serve(path, rec, mode="kde"):
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
    calls = {}  # our serial -> what we asked

    def send(mtype, fields, sig="", body=b""):
        ser[0] += 1
        f = fields + ([(8, "g", sig)] if sig else [])
        c.sendall(message(mtype, ser[0], f, body))
        return ser[0]

    def reply(m, sig="", body=b"", sender="org.freedesktop.DBus"):
        send(2, [(5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", sender)], sig, body)

    def error(m, name):
        send(3, [(4, "s", name), (5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", "org.freedesktop.DBus")])

    def ask(path, iface, member, sig="", body=b"", what=None, sender=PANEL):
        s = send(1, [(1, "o", path), (2, "s", iface), (3, "s", member), (6, "s", ":1.42"), (7, "s", sender)], sig, body)
        calls[s] = what or member

    def attack():
        # another client of the bus: a NameOwnerChanged of its own (unicast, past the match rule)
        # naming itself the watcher, then the calls a panel would make
        send(4, [(1, "o", "/org/freedesktop/DBus"), (2, "s", "org.freedesktop.DBus"), (3, "s", "NameOwnerChanged"),
                 (6, "s", ":1.42"), (7, "s", EVIL)], "sss", W().all("sss", [WATCHER, KDED, EVIL]))
        ask("/StatusNotifierItem", "org.kde.StatusNotifierItem", "SecondaryActivate", "ii", W().all("ii", [1, 2]), "attack", EVIL)
        ask("/MenuBar", "com.canonical.dbusmenu", "Event", "isvu", W().all("isvu", [1, "clicked", ("i", 0), 0]), "attack", EVIL)
        ask("/StatusNotifierItem", "org.kde.StatusNotifierItem", "ProvideXdgActivationToken", "s", W().all("s", ["evil-token"]), "attack", EVIL)

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
            elif member == "GetNameOwner":
                # the watcher runs (this fake plays it): the symbol is supported; KDE: kded owns
                # it and plasmashell a host name, GNOME: the panel owns the watcher
                rec.setdefault("has_owner", []).append(body[0])
                owner = {WATCHER: KDED if mode == "kde" else PANEL, HOST: PANEL, EVIL_HOST: EVIL}.get(body[0])
                if owner and (body[0] != HOST or mode == "kde"):
                    reply(m, "s", W().all("s", [owner]))
                else:
                    error(m, "org.freedesktop.DBus.Error.NameHasNoOwner")
            elif member == "ListNames":
                rec["list_names"] = rec.get("list_names", 0) + 1
                # a forged answer first (from another client, with the serial of the call)
                reply(m, "as", W().all("as", [[EVIL_HOST]]), sender=EVIL)
                names = ["org.freedesktop.DBus", ":1.42", KDED, PANEL, EVIL, WATCHER, "org.freedesktop.Notifications",
                         "org.kde.StatusNotifierHostile", "org.kde.plasmashell"] + ([HOST] if mode == "kde" else [])
                reply(m, "as", W().all("as", [names]))
            elif member in ("RequestName", "ReleaseName"):
                rec[member] = body[0]
                reply(m, "u", W().all("u", [1]))
            elif member == "RegisterStatusNotifierItem":
                rec["registered"] = body[0]
                reply(m)
                attack()
                # the panel looks at the item
                ask("/StatusNotifierItem", "org.freedesktop.DBus.Properties", "GetAll", "s",
                    W().all("s", ["org.kde.StatusNotifierItem"]))
                ask("/MenuBar", "com.canonical.dbusmenu", "GetLayout", "iias", W().all("iias", [0, -1, []]))
            elif member == "Notify":
                rec["notify"] = body
                # the notification server :1.3 answers: its signals for the note count
                reply(m, "u", W().all("u", [77]), sender=":1.3")
                # first another client (:1.66) claims the note was closed -- it must not count
                # (else the click is lost and NoteClosed arrives) -- then the server's own signals
                for snd, mem, s, vals in ((":1.66", "NotificationClosed", "uu", [77, 2]),
                                          (":1.3", "ActionInvoked", "us", [77, "default"]),
                                          (":1.3", "NotificationClosed", "uu", [77, 2])):
                    send(4, [(1, "o", "/org/freedesktop/Notifications"), (2, "s", "org.freedesktop.Notifications"),
                             (3, "s", mem), (7, "s", snd)], s, W().all(s, vals))
            else:
                reply(m)
        elif m["type"] in (2, 3):
            what = calls.pop(fl.get(5, 0), None)
            if what == "attack":
                continue
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


def run(exe, mode):
    rec = {}
    with tempfile.TemporaryDirectory() as d:
        sock = os.path.join(d, "bus")
        t = threading.Thread(target=serve, args=(listen_unix(sock, 1), rec, mode), daemon=True)
        t.start()
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}")
        r = subprocess.run([str(exe), "--test", "tray-bus"], env=env, capture_output=True, text=True, timeout=60)
        t.join(5)
    print(f"-- {mode}")
    print(r.stdout.strip())
    check(f"{mode}: plattest --test tray-bus passed", r.returncode == 0, r.stderr.strip())
    check(f"{mode}: who owns the host names asked once (ListNames), the forged host name never",
          rec.get("list_names") == 1 and EVIL_HOST not in rec.get("has_owner", []), rec)
    return rec


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"
    run(exe, "gnome")
    rec = run(exe, "kde")
    check("SASL EXTERNAL", rec.get("auth"), rec)
    check("matches for notifications and the watcher",
          any("org.freedesktop.Notifications" in x for x in rec.get("matches", []))
          and any("StatusNotifierWatcher" in x for x in rec.get("matches", [])), rec.get("matches"))
    name = rec.get("RequestName", "")
    check("item name org.kde.StatusNotifierItem-<pid>-1", name.startswith("org.kde.StatusNotifierItem-") and name.endswith("-1"), name)
    check("registered with the watcher", rec.get("registered") == name, rec.get("registered"))
    check("asked whether the watcher runs (tray_supported)", "org.kde.StatusNotifierWatcher" in rec.get("has_owner", []),
          rec.get("has_owner"))
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
