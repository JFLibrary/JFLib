#!/usr/bin/env python3
"""settings_bus.py -- plat's system settings on Linux (lib/plat/linux_settings.rs) against a
fake settings portal.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/settings_bus.py [--rs build]

The fake session bus answers org.freedesktop.portal.Settings.ReadAll with appearance
(color-scheme 1, accent-color (0.2, 0.4, 0.6)), interface (enable-animations false,
text-scaling-factor 1.25, cursor-blink-time 1200, gtk-theme Adwaita, font-antialiasing rgba,
font-hinting medium, font-rgba-order bgr), a11y (high-contrast
true, wrapped in two variants) and mouse (double-click 400); plattest --test settings-bus
checks the result.  The fake checks the namespaces it was asked for.  Exit code 0 = passed.
"""
import argparse, os, socket, subprocess, sys, tempfile, threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, message, parse  # noqa: E402

fails = 0

SETTINGS = {
    "org.freedesktop.appearance": {"color-scheme": ("u", 1), "accent-color": ("(ddd)", (0.2, 0.4, 0.6))},
    "org.gnome.desktop.interface": {"enable-animations": ("b", False), "text-scaling-factor": ("d", 1.25),
                                    "cursor-blink-time": ("i", 1200), "gtk-theme": ("s", "Adwaita"),
                                    "font-antialiasing": ("s", "rgba"), "font-hinting": ("s", "medium"),
                                    "font-rgba-order": ("s", "bgr")},
    "org.gnome.desktop.a11y.interface": {"high-contrast": ("v", ("b", True))},
    "org.gnome.desktop.peripherals.mouse": {"double-click": ("i", 400)},
}


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def serve(path, rec):
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(4)
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

    def reply(m, sig="", body=b""):
        ser[0] += 1
        f = [(5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", "org.freedesktop.DBus")] + ([(8, "g", sig)] if sig else [])
        c.sendall(message(2, ser[0], f, body))

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
            continue
        if member == "Hello":
            reply(m, "s", W().all("s", [":1.42"]))
        elif member == "ReadAll":
            rec["readall"] = (fl.get(6), fl.get(1), fl.get(2), body)
            asked = body[0] if body else []
            reply(m, "a{sa{sv}}", W().all("a{sa{sv}}", [{k: v for k, v in SETTINGS.items() if k in asked}]))
        else:
            reply(m)


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
        for k in list(env):
            if k.startswith("jfgui_"):
                env.pop(k)
        r = subprocess.run([str(exe), "--test", "settings-bus"], env=env, capture_output=True, text=True, timeout=60)
        t.join(5)
    print(r.stdout.strip())
    check("plattest --test settings-bus passed", r.returncode == 0, r.stderr.strip())
    ra = rec.get("readall")
    check("ReadAll on the portal with the four namespaces",
          ra is not None and ra[0] == "org.freedesktop.portal.Desktop" and ra[1] == "/org/freedesktop/portal/desktop"
          and ra[2] == "org.freedesktop.portal.Settings" and sorted(ra[3][0]) == sorted(SETTINGS), ra)
    print("settings_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
