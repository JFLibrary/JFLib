#!/usr/bin/env python3
"""life_bus.py -- plat's lifecycle watcher on Linux (lib/plat/linux_life.rs) between a fake
system bus and a fake session bus.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/life_bus.py [--rs build]

The system bus plays logind: after the AddMatch it sends PrepareForSleep(true) and
PrepareForSleep(false).  The session bus plays the settings portal: SettingChanged for
color-scheme (u), an unrelated key (s) and text-scaling-factor (d).  plattest --test life-bus
must see E_POWER 1 / 0 and E_SETTINGS SET_THEME / SET_FONTS; then this script sends SIGTERM
and plattest must get E_SESSION_END and end normally.  Exit code 0 = passed.
"""
import argparse, os, signal, socket, subprocess, sys, tempfile, threading, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, message, parse  # noqa: E402

fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def serve(path, rec, signals):
    """a bus that answers Hello / AddMatch and sends `signals` after the first AddMatch"""
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

    def send(mtype, fields, sig="", body=b""):
        ser[0] += 1
        f = fields + ([(8, "g", sig)] if sig else [])
        c.sendall(message(mtype, ser[0], f, body))

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
        send(2, [(5, "u", m["serial"]), (6, "s", ":1.42"), (7, "s", "org.freedesktop.DBus")],
             "s" if member == "Hello" else "", W().all("s", [":1.42"]) if member == "Hello" else b"")
        if member == "AddMatch":
            rec.setdefault("matches", []).append(body[0])
            if not rec.get("sent"):
                rec["sent"] = True
                for path, iface, mem, s, vals in signals:
                    send(4, [(1, "o", path), (2, "s", iface), (3, "s", mem), (7, "s", ":1.9")], s, W().all(s, vals))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"
    sysrec, sesrec = {}, {}
    login = "/org/freedesktop/login1"
    sleep = [(login, "org.freedesktop.login1.Manager", "PrepareForSleep", "b", [True]),
             (login, "org.freedesktop.login1.Manager", "PrepareForSleep", "b", [False])]
    desk = "/org/freedesktop/portal/desktop"
    sett = "org.freedesktop.portal.Settings"
    settings = [(desk, sett, "SettingChanged", "ssv", ["org.freedesktop.appearance", "color-scheme", ("u", 1)]),
                (desk, sett, "SettingChanged", "ssv", ["org.example.app", "whatever", ("s", "x")]),
                (desk, sett, "SettingChanged", "ssv", ["org.gnome.desktop.interface", "text-scaling-factor", ("d", 1.25)])]
    with tempfile.TemporaryDirectory() as d:
        sp, ep = os.path.join(d, "system"), os.path.join(d, "session")
        ts = [threading.Thread(target=serve, args=(sp, sysrec, sleep), daemon=True),
              threading.Thread(target=serve, args=(ep, sesrec, settings), daemon=True)]
        for t in ts:
            t.start()
        env = dict(os.environ, DBUS_SYSTEM_BUS_ADDRESS=f"unix:path={sp}", DBUS_SESSION_BUS_ADDRESS=f"unix:path={ep}")
        env.pop("jfgui_LIFE", None)
        p = subprocess.Popen([str(exe), "--test", "life-bus"], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        out = []
        killed = False
        while True:
            ln = p.stdout.readline()
            if not ln:
                break
            out.append(ln.rstrip("\n"))
            if "waiting for SIGTERM" in ln and not killed:
                time.sleep(0.2)
                os.kill(p.pid, signal.SIGTERM)
                killed = True
        err = p.stderr.read()
        code = p.wait(30)
    print("\n".join(out))
    check("plattest --test life-bus passed (and outlived SIGTERM)", code == 0 and killed, f"exit {code}, {err.strip()}")
    check("SASL on both buses", sysrec.get("auth") and sesrec.get("auth"), (sysrec, sesrec))
    check("a match for logind's PrepareForSleep",
          any("org.freedesktop.login1.Manager" in m and "PrepareForSleep" in m for m in sysrec.get("matches", [])), sysrec.get("matches"))
    check("a match for the portal's SettingChanged",
          any("org.freedesktop.portal.Settings" in m and "SettingChanged" in m for m in sesrec.get("matches", [])), sesrec.get("matches"))
    print("life_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
