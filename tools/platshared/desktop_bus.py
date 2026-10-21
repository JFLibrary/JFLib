#!/usr/bin/env python3
"""desktop_bus.py -- plat's desktop integration on Linux (lib/plat/linux_power.rs,
linux_taskbar.rs, linux_autostart.rs) against a fake session bus.  Run inside WSL / Linux from
the repository root:

  python3 tools/platshared/desktop_bus.py [--rs build] [--only power|taskbar|autostart]

power      plattest --test power-bus --mode portal: org.freedesktop.portal.Inhibit.Inhibit with
           flags 4 (suspend) and 12 (idle implies suspend), the reasons and handle tokens, then
           Request.Close on both returned paths (idle first), one connection, closed after the
           last guard (before the process ends; this portal sends no Response on success).
           --mode idleonly: a portal backend like xdg-desktop-portal-gtk that answers, then
           refuses suspend with Response(2) and accepts idle with Response(0): suspend
           goes to PowerManagement, idle is asked again with flags 8 (+ PowerManagement as best
           effort).  --mode fallback: the portal is missing (ServiceUnknown), so
           org.freedesktop.ScreenSaver.Inhibit (idle) and org.freedesktop.PowerManagement
           .Inhibit (suspend) hand out cookies, released with UnInhibit.  --mode none: nothing
           answers, every request is None, nothing released.
taskbar    plattest --test taskbar-bus: the com.canonical.Unity.LauncherEntry.Update signals
           (application://plattest-taskbar.desktop; progress (d), progress-visible, count (x),
           count-visible) of normal 25 %, paused 50 %, badge 3, error 75 %, indeterminate, none,
           badge 0; after the fourth the fake calls Query on the sender: answered with the
           state of the next update (indeterminate, count 3).
autostart  plattest --test autostart-bus --mode grant: Background.RequestBackground (handle
           token, reason, autostart, commandline [exe, --hidden, "a b"] only with arguments,
           dbus-activatable false), the AddMatch for the Request's Response, Response 0 with
           autostart as asked.  --mode deny: Response 1 -> Dismissed.  --mode none: no portal
           -> Unavailable.
Exit code 0 = passed.
"""
import argparse, os, socket, subprocess, sys, tempfile, threading, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, message, parse  # noqa: E402

fails = 0
PORTAL = "org.freedesktop.portal.Desktop"
UNKNOWN = "org.freedesktop.DBus.Error.ServiceUnknown"


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


class Conn:
    """one client of the fake bus: its socket and unique name"""

    def __init__(self, c, unique):
        self.c, self.unique, self.ser, self.lock = c, unique, 100, threading.Lock()

    def send(self, mtype, fields, sig="", body=b""):
        with self.lock:
            self.ser += 1
            self.c.sendall(message(mtype, self.ser, fields + ([(8, "g", sig)] if sig else []), body))
            return self.ser

    def reply(self, m, sig="", body=b""):
        if not m["raw"][2] & 1:  # unless NO_REPLY_EXPECTED
            self.send(2, [(5, "u", m["serial"]), (6, "s", self.unique), (7, "s", "org.freedesktop.DBus")], sig, body)

    def error(self, m, name, text="no"):
        self.send(3, [(4, "s", name), (5, "u", m["serial"]), (6, "s", self.unique), (7, "s", "org.freedesktop.DBus")], "s", W().all("s", [text]))

    def signal(self, path, iface, member, sig, body):
        self.send(4, [(1, "o", path), (2, "s", iface), (3, "s", member), (6, "s", self.unique), (7, "s", ":1.7")], sig, body)

    def call(self, path, iface, member):
        return self.send(1, [(1, "o", path), (2, "s", iface), (3, "s", member), (6, "s", self.unique), (7, "s", ":1.7")])


class Bus:
    """a session bus at path: every connection gets a thread and the unique name :1.<42 + n>;
    handler(conn, msg, body) answers the calls (Hello is answered here).  rec: calls (unique,
    dest, path, iface, member, body), signals (unique, path, iface, member, body), returns
    (unique, reply_to, sig, body), eof (unique, time), conns"""

    def __init__(self, path, handler):
        self.handler, self.lock, self.conns = handler, threading.Lock(), {}
        self.rec = {"calls": [], "signals": [], "returns": [], "eof": [], "conns": 0}
        self.srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.srv.bind(path)
        self.srv.listen(8)
        self.srv.settimeout(0.2)
        self.stop = False
        self.t = threading.Thread(target=self.accept, daemon=True)
        self.t.start()

    def accept(self):
        while not self.stop:
            try:
                c, _ = self.srv.accept()
            except (socket.timeout, OSError):
                continue
            with self.lock:
                unique = f":1.{42 + self.rec['conns']}"
                self.rec["conns"] += 1
            threading.Thread(target=self.serve, args=(c, unique), daemon=True).start()

    def close(self):
        self.stop = True
        self.t.join(2)
        self.srv.close()

    def log(self, key, item):
        with self.lock:
            self.rec[key].append(item)

    def serve(self, c, unique):
        c.settimeout(20)
        buf = b""
        while b"BEGIN\r\n" not in buf:
            d = c.recv(4096)
            if not d:
                return
            buf += d
            if b"AUTH EXTERNAL" in buf and b"\r\n" in buf.split(b"AUTH", 1)[1] and unique not in self.conns:
                self.conns[unique] = Conn(c, unique)
                c.sendall(b"OK 0123456789abcdef0123456789abcdef\r\n")
        buf = buf.split(b"BEGIN\r\n", 1)[1]
        conn = self.conns[unique]
        while True:
            m, buf = parse(buf)
            if m is None:
                try:
                    d = c.recv(65536)
                except (socket.timeout, OSError):
                    d = b""
                if not d:
                    self.log("eof", (unique, time.time()))
                    return
                buf += d
                continue
            fl = m["fields"]
            sig = fl.get(8, "")
            body = R(m["body"]).all(sig) if sig else []
            if m["type"] == 4:
                self.log("signals", (unique, fl.get(1), fl.get(2), fl.get(3), body))
            elif m["type"] == 2:
                self.log("returns", (unique, fl.get(5), sig, body))
            elif m["type"] == 1 and fl.get(3) == "Hello":
                conn.reply(m, "s", W().all("s", [unique]))
            elif m["type"] == 1:
                self.log("calls", (unique, fl.get(6), fl.get(1), fl.get(2), fl.get(3), body))
                self.handler(conn, m, body)


def run(exe, args, sock):
    """plattest on the fake bus -> (result, the time it ended)"""
    env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}")
    for k in list(env):
        if k.startswith("jfgui_"):
            env.pop(k)
    r = subprocess.run([str(exe)] + args, env=env, capture_output=True, text=True, timeout=120)
    end = time.time()
    print(r.stdout.strip())
    if r.stderr.strip():
        print(r.stderr.strip())
    return r, end


def members(rec, member):
    return [c for c in rec["calls"] if c[4] == member]


def request_path(conn, token):
    return f"/org/freedesktop/portal/desktop/request/{conn.unique[1:].replace('.', '_')}/{token}"


# ------------------------------------------------------------------ power
def power(exe, d):
    for mode in ("portal", "idleonly", "fallback", "none"):
        cookies = {"s": iter(range(11, 99)), "p": iter(range(21, 99))}

        def handler(conn, m, body, mode=mode, cookies=cookies):
            dest, member = m["fields"].get(6), m["fields"].get(3, "")
            if dest == PORTAL and member == "Inhibit" and mode in ("portal", "idleonly"):
                path = request_path(conn, body[2]["handle_token"][1])
                conn.reply(m, "o", W().all("o", [path]))
                if mode == "idleonly":  # the backend's verdict after the answer (as xdg-desktop-portal 1.18)
                    time.sleep(0.05)
                    conn.signal(path, "org.freedesktop.portal.Request", "Response", "ua{sv}", W().all("ua{sv}", [2 if body[1] & 4 else 0, {}]))
            elif dest == "org.freedesktop.ScreenSaver" and member == "Inhibit" and mode == "fallback":
                conn.reply(m, "u", W().all("u", [next(cookies["s"])]))
            elif dest == "org.freedesktop.PowerManagement" and member == "Inhibit" and mode in ("fallback", "idleonly"):
                conn.reply(m, "u", W().all("u", [next(cookies["p"])]))
            elif member in ("Close", "UnInhibit", "AddMatch", "RemoveMatch"):
                conn.reply(m)
            else:
                conn.error(m, UNKNOWN, "nobody here")

        sock = os.path.join(d, f"power-{mode}")
        bus = Bus(sock, handler)
        r, end = run(exe, ["--test", "power-bus", "--mode", mode], sock)
        time.sleep(0.3)
        bus.close()
        rec = bus.rec
        check(f"power ({mode}): plattest --test power-bus passed", r.returncode == 0, r.returncode)
        inh = [c for c in members(rec, "Inhibit") if c[1] == PORTAL]
        if mode == "portal":
            ok = len(inh) == 2 and [c[5][1] for c in inh] == [4, 12] and all(
                c[2] == "/org/freedesktop/portal/desktop" and c[3] == "org.freedesktop.portal.Inhibit" and c[5][0] == "" for c in inh)
            check("power (portal): Inhibit(\"\", 4 / 12, options) on the portal", ok, inh)
            opts = [c[5][2] for c in inh]
            check("power (portal): the reasons and handle tokens",
                  [o.get("reason", ("s", ""))[1] for o in opts] == ["plattest: export", "plattest: film"]
                  and all(o.get("handle_token", ("s", ""))[1].startswith("jfgui_inhibit") for o in opts), opts)
            closes = members(rec, "Close")
            paths = [f"/org/freedesktop/portal/desktop/request/1_42/{o['handle_token'][1]}" for o in opts]
            check("power (portal): Request.Close on both paths, idle first", [c[2] for c in closes] == paths[::-1]
                  and all(c[3] == "org.freedesktop.portal.Request" and c[1] == PORTAL for c in closes), closes)
            check("power (portal): one connection, closed after the last guard (before the process ended)",
                  rec["conns"] == 1 and len(rec["eof"]) == 1 and rec["eof"][0][1] < end - 0.1, (rec["conns"], rec["eof"], end))
            am = [c[5][0] for c in members(rec, "AddMatch")]
            check("power (portal): AddMatch for each request's Response", len(am) == 2 and all(
                f"path='{p}'" in r and "member='Response'" in r for p, r in zip(paths, am)), am)
        elif mode == "idleonly":
            check("power (idleonly): the portal asked with 4, 12, then 8", [c[5][1] for c in inh] == [4, 12, 8], inh)
            pm = [c for c in members(rec, "Inhibit") if c[1] == "org.freedesktop.PowerManagement"]
            check("power (idleonly): PowerManagement for suspend and beside idle", [c[5][1] for c in pm] == ["plattest: export", "plattest: film"], pm)
            idle = request_path(Conn(None, ":1.42"), inh[2][5][2]["handle_token"][1]) if len(inh) == 3 else ""
            rel = [(c[4], c[2], c[5]) for c in rec["calls"] if c[4] in ("Close", "UnInhibit")]
            want = [("Close", idle, []), ("UnInhibit", "/org/freedesktop/PowerManagement/Inhibit", [22]),
                    ("UnInhibit", "/org/freedesktop/PowerManagement/Inhibit", [21])]
            check("power (idleonly): released: the idle request and cookie 22, then cookie 21", rel == want, rel)
        elif mode == "fallback":
            ss = [c for c in members(rec, "Inhibit") if c[1] == "org.freedesktop.ScreenSaver"]
            pm = [c for c in members(rec, "Inhibit") if c[1] == "org.freedesktop.PowerManagement"]
            check("power (fallback): the portal was asked first (4, 12, then 8 for idle alone)", [c[5][1] for c in inh] == [4, 12, 8], inh)
            check("power (fallback): ScreenSaver.Inhibit(app, reason) for idle", len(ss) == 1 and ss[0][2] == "/org/freedesktop/ScreenSaver"
                  and ss[0][5] == ["plattest", "plattest: film"], ss)
            check("power (fallback): PowerManagement.Inhibit for both", len(pm) == 2 and all(
                c[2] == "/org/freedesktop/PowerManagement/Inhibit" and c[3] == "org.freedesktop.PowerManagement.Inhibit" for c in pm)
                  and [c[5][1] for c in pm] == ["plattest: export", "plattest: film"], pm)
            un = [(c[1], c[5]) for c in members(rec, "UnInhibit")]
            check("power (fallback): UnInhibit of the cookies (11 + 22, then 21)",
                  un == [("org.freedesktop.ScreenSaver", [11]), ("org.freedesktop.PowerManagement", [22]), ("org.freedesktop.PowerManagement", [21])], un)
        else:
            check("power (none): nothing released", not members(rec, "Close") and not members(rec, "UnInhibit"), rec["calls"])
            check("power (none): no connection kept after a failure", rec["conns"] >= 1 and len(rec["eof"]) == rec["conns"]
                  and all(t < end - 0.1 for _, t in rec["eof"]), (rec["conns"], rec["eof"], end))


# ------------------------------------------------------------------ taskbar
def taskbar(exe, d):
    sock = os.path.join(d, "taskbar")
    bus = Bus(sock, lambda conn, m, body: conn.reply(m) if m["fields"].get(3) == "AddMatch" else conn.error(m, UNKNOWN))
    asked = {}
    stop = threading.Event()

    def asker():
        # after the fourth Update: ask the sender, as a dock that starts late would
        while not stop.is_set():
            ups = [s for s in bus.rec["signals"] if s[3] == "Update"]
            if len(ups) >= 4:
                asked["serial"] = bus.conns[ups[0][0]].call(ups[0][1], "com.canonical.Unity.LauncherEntry", "Query")
                return
            time.sleep(0.01)

    t = threading.Thread(target=asker, daemon=True)
    t.start()
    r, _ = run(exe, ["--test", "taskbar-bus"], sock)
    stop.set()
    time.sleep(0.2)
    bus.close()
    rec = bus.rec
    check("taskbar: plattest --test taskbar-bus passed", r.returncode == 0, r.returncode)
    ups = [s for s in rec["signals"] if s[2] == "com.canonical.Unity.LauncherEntry" and s[3] == "Update"]
    check("taskbar: seven Update signals", len(ups) == 7, len(ups))
    uri = "application://plattest-taskbar.desktop"
    check("taskbar: from /com/canonical/unity/launcherentry/<n> for the desktop id",
          all(u[1].startswith("/com/canonical/unity/launcherentry/") and u[4][0] == uri for u in ups), [(u[1], u[4][0]) for u in ups])

    def props(p):
        return (round(p["progress"][1], 3), p["progress-visible"][1], p["count"][1], p["count-visible"][1])

    want = [(0.25, 1, 0, 0), (0.5, 1, 0, 0), (0.5, 1, 3, 1), (0.75, 1, 3, 1), (0.0, 0, 3, 1), (0.0, 0, 3, 1), (0.0, 0, 0, 0)]
    got = [props(u[4][1]) for u in ups]
    check("taskbar: progress / progress-visible / count / count-visible of each step", got == want, got)
    types = {(u[4][1]["count"][0], u[4][1]["progress"][0]) for u in ups}
    check("taskbar: count is an int64 (x), progress a double (d)", types == {("x", "d")}, types)
    ans = [x for x in rec["returns"] if x[1] == asked.get("serial")]
    # answered when the connection is used next: the fifth step has just set its state
    ok = len(ans) == 1 and ans[0][2] == "sa{sv}" and ans[0][3][0] == uri and props(ans[0][3][1]) == (0.0, 0, 3, 1)
    check("taskbar: Query answered with the current state (indeterminate, count 3)", ok, ans)


# ------------------------------------------------------------------ autostart
def autostart(exe, d):
    for mode in ("grant", "deny", "none"):
        def handler(conn, m, body, mode=mode):
            dest, member = m["fields"].get(6), m["fields"].get(3, "")
            if member == "AddMatch":
                conn.reply(m)
            elif dest == PORTAL and member == "RequestBackground" and mode != "none":
                opts = body[1]
                path = request_path(conn, opts["handle_token"][1])
                conn.reply(m, "o", W().all("o", [path]))
                res = {"background": ("b", 1), "autostart": ("b", opts["autostart"][1])} if mode == "grant" else {}
                conn.signal(path, "org.freedesktop.portal.Request", "Response", "ua{sv}", W().all("ua{sv}", [0 if mode == "grant" else 1, res]))
            else:
                conn.error(m, UNKNOWN, "no portal")

        sock = os.path.join(d, f"autostart-{mode}")
        bus = Bus(sock, handler)
        r, _ = run(exe, ["--test", "autostart-bus", "--mode", mode], sock)
        time.sleep(0.2)
        bus.close()
        rec = bus.rec
        check(f"autostart ({mode}): plattest --test autostart-bus passed", r.returncode == 0, r.returncode)
        reqs = members(rec, "RequestBackground")
        if mode == "grant":
            check("autostart (grant): three requests on org.freedesktop.portal.Background", len(reqs) == 3 and all(
                c[1] == PORTAL and c[2] == "/org/freedesktop/portal/desktop" and c[3] == "org.freedesktop.portal.Background"
                and c[5][0] == "" for c in reqs), reqs)
            o = [c[5][1] for c in reqs]
            check("autostart (grant): autostart true, true, false", [x["autostart"][1] for x in o] == [1, 1, 0], o)
            cl = o[0].get("commandline", ("as", []))[1] if o else []
            check("autostart (grant): commandline [exe, --hidden, \"a b\"] only with arguments",
                  len(o) == 3 and len(cl) == 3 and cl[0].endswith("/plattest") and cl[1:] == ["--hidden", "a b"]
                  and "commandline" not in o[1] and "commandline" not in o[2], o)
            check("autostart (grant): handle_token, reason, dbus-activatable false",
                  all(x["handle_token"][1].startswith("jfgui_bg") and x["reason"][1] and x["dbus-activatable"][1] == 0 for x in o), o)
            am = [c for c in members(rec, "AddMatch") if "Response" in c[5][0]]
            check("autostart (grant): AddMatch for each Request's Response path", len(am) == 3 and all(
                "path='/org/freedesktop/portal/desktop/request/1_" in c[5][0] for c in am), am)
        else:
            check(f"autostart ({mode}): asked once", len(reqs) == 1, reqs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    ap.add_argument("--only", choices=["power", "taskbar", "autostart"])
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"
    with tempfile.TemporaryDirectory() as d:
        for name, f in (("power", power), ("taskbar", taskbar), ("autostart", autostart)):
            if a.only in (None, name):
                f(exe, d)
    print("desktop_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
