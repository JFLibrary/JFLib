#!/usr/bin/env python3
"""screen_bus.py -- plat's screen portals on Linux (lib/plat/linux_screen.rs, linux_hotkey.rs)
against a fake session bus.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/screen_bus.py [--rs build] [--only grant|deny|none]

grant  plattest --test screen-bus --mode grant: the portals' version properties
       (Properties.Get: Screenshot 2, GlobalShortcuts 1); Screenshot.PickColor("", {handle_token})
       answered with the colour (0.2, 0.4, 0.6) -> #336699; Screenshot.Screenshot("",
       {handle_token, modal true, interactive false}) answered with the uri of a 3 x 2 PNG
       ("shot 1.png", %20 in the uri); GlobalShortcuts: CreateSession ({handle_token,
       session_handle_token}) -> session_handle, BindShortcuts (session, [(id, {description,
       preferred_trigger})], "", {handle_token}) of 7 "Ctrl+Alt+K", 8 "Ctrl+Alt+J", 9 "Super+F12"
       binds 7 and 8 only (9 comes back Refused), then Activated(session, "9") and (session, "7")
       (only 7 is Pressed); unregister 8: the session closed (Session.Close) and 7 bound in a new
       one; clear: the last session closed and the connection with it.
deny   PickColor Response 1 (Cancelled), Screenshot Response 2 (Denied), BindShortcuts Response
       1 (Refused, Dismissed).
none   no portal at all (ServiceUnknown): nothing available, Unavailable.
Exit code 0 = passed.
"""
import argparse, os, sys, tempfile, threading, time, urllib.parse, zlib, struct
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import W  # noqa: E402
from desktop_bus import Bus, members, request_path  # noqa: E402

import subprocess

fails = 0
PORTAL = "org.freedesktop.portal.Desktop"
SHOT = "org.freedesktop.portal.Screenshot"
GS = "org.freedesktop.portal.GlobalShortcuts"
UNKNOWN = "org.freedesktop.DBus.Error.ServiceUnknown"


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def png(w, h, rgb):
    """a w x h RGB PNG of one colour"""
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    raw = b"".join(b"\0" + bytes(rgb) * w for _ in range(h))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def run(exe, args, sock):
    env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}")
    for k in list(env):
        if k.startswith("jfgui_"):
            env.pop(k)
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    r = subprocess.run([str(exe)] + args, env=env, capture_output=True, text=True, timeout=120)
    end = time.time()
    print(r.stdout.strip())
    if r.stderr.strip():
        print(r.stderr.strip())
    return r, end


def scenario(exe, d, mode):
    shot_file = os.path.join(d, "shot 1.png")
    sessions = []  # (path, ids)

    def handler(conn, m, body, mode=mode):
        dest, path, iface, member = m["fields"].get(6), m["fields"].get(1), m["fields"].get(2), m["fields"].get(3, "")
        if member in ("AddMatch", "RemoveMatch"):
            conn.reply(m)
            return
        if mode == "none" or dest != PORTAL:
            conn.error(m, UNKNOWN, "no portal")
            return
        if iface == "org.freedesktop.DBus.Properties" and member == "Get":
            ver = {SHOT: 2, GS: 1}.get(body[0])
            if ver is None:
                conn.error(m, "org.freedesktop.DBus.Error.UnknownInterface", "no such interface")
            else:
                conn.reply(m, "v", W().all("v", [("u", ver)]))
            return
        if iface == "org.freedesktop.portal.Session" and member == "Close":
            conn.reply(m)
            return
        opts = body[-1] if body and isinstance(body[-1], dict) else {}
        token = opts.get("handle_token", ("s", "none"))[1]
        req = request_path(conn, token)
        conn.reply(m, "o", W().all("o", [req]))
        time.sleep(0.05)

        def response(code, res):
            conn.signal(req, "org.freedesktop.portal.Request", "Response", "ua{sv}", W().all("ua{sv}", [code, res]))

        if iface == SHOT and member == "PickColor":
            response(0, {"color": ("(ddd)", (0.2, 0.4, 0.6))}) if mode == "grant" else response(1, {})
        elif iface == SHOT and member == "Screenshot":
            if mode == "grant":
                with open(shot_file, "wb") as f:
                    f.write(png(3, 2, (10, 20, 30)))
                response(0, {"uri": ("s", "file://" + urllib.parse.quote(shot_file))})
            else:
                response(2, {})
        elif iface == GS and member == "CreateSession":
            st = opts.get("session_handle_token", ("s", "s"))[1]
            sp = f"/org/freedesktop/portal/desktop/session/{conn.unique[1:].replace('.', '_')}/{st}"
            response(0, {"session_handle": ("s", sp)})
        elif iface == GS and member == "BindShortcuts":
            session, shortcuts = body[0], body[1]
            ids = [s[0] for s in shortcuts]
            sessions.append((session, ids))
            if mode != "grant":
                response(1, {})
                return
            bound = [(i, {"description": p["description"], "trigger_description": ("s", "Strg+Alt+" + i)}) for i, p in shortcuts if i != "9"]
            response(0, {"shortcuts": ("a(sa{sv})", bound)})
            if "9" in ids:
                def activate():
                    time.sleep(0.2)
                    for i in ("9", "7"):
                        conn.signal("/org/freedesktop/portal/desktop", GS, "Activated", "osta{sv}", W().all("osta{sv}", [session, i, 123456, {}]))
                threading.Thread(target=activate, daemon=True).start()
        else:
            conn.error(m, "org.freedesktop.DBus.Error.UnknownMethod", "no")

    sock = os.path.join(d, f"screen-{mode}")
    bus = Bus(sock, handler)
    r, end = run(exe, ["--test", "screen-bus", "--mode", mode], sock)
    time.sleep(0.3)
    bus.close()
    rec = bus.rec
    check(f"{mode}: plattest --test screen-bus passed", r.returncode == 0, r.returncode)
    if mode == "none":
        check("none: no shortcut session asked", not members(rec, "CreateSession"), rec["calls"])
        return
    gets = [c[5] for c in members(rec, "Get")]
    check(f"{mode}: the versions asked (Properties.Get)", [GS, "version"] in gets and ([SHOT, "version"] in gets or mode == "deny"), gets)
    pick = members(rec, "PickColor")
    check(f"{mode}: PickColor(\"\", {{handle_token}}) on the Screenshot portal", len(pick) == 1 and pick[0][1] == PORTAL and pick[0][2] == "/org/freedesktop/portal/desktop"
          and pick[0][3] == SHOT and pick[0][5][0] == "" and pick[0][5][1]["handle_token"][1].startswith("jfgui_pick"), pick)
    shot = members(rec, "Screenshot")
    check(f"{mode}: Screenshot(\"\", {{handle_token, modal, interactive false}})", len(shot) == 1 and shot[0][5][0] == ""
          and shot[0][5][1]["modal"][1] == 1 and shot[0][5][1]["interactive"][1] == 0 and shot[0][5][1]["handle_token"][1].startswith("jfgui_shot"), shot)
    am = [c[5][0] for c in members(rec, "AddMatch")]
    check(f"{mode}: AddMatch for the Responses and Activated", any("member='Response'" in a for a in am) and any("member='Activated'" in a for a in am), am)
    cs = members(rec, "CreateSession")
    check(f"{mode}: CreateSession with handle_token and session_handle_token", cs and all(
        c[3] == GS and c[5][0]["handle_token"][1].startswith("jfgui_hk") and c[5][0]["session_handle_token"][1].startswith("jfgui_hks") for c in cs), cs)
    binds = members(rec, "BindShortcuts")
    if mode == "deny":
        check("deny: BindShortcuts once (then Refused)", len(binds) == 1, binds)
        return
    full = [b for b in binds if [s[0] for s in b[5][1]] == ["7", "8", "9"]]
    check("grant: 7, 8, 9 bound together with descriptions and preferred triggers", len(full) >= 1 and full[-1][5][1] == [
        ("7", {"description": ("s", "Zeigen"), "preferred_trigger": ("s", "CTRL+ALT+k")}),
        ("8", {"description": ("s", "Verstecken"), "preferred_trigger": ("s", "CTRL+ALT+j")}),
        ("9", {"description": ("s", "Nicht gebunden"), "preferred_trigger": ("s", "LOGO+F12")})] and full[-1][5][2] == "", full[-1:] if full else binds)
    last = binds[-1] if binds else None
    check("grant: after unregister 8 only 7 is bound, in a new session", last is not None and [s[0] for s in last[5][1]] == ["7"]
          and last[5][0] != full[-1][5][0] if full else False, binds)
    closes = [c[2] for c in members(rec, "Close")]
    check("grant: every session closed (unregister, clear)", full and full[-1][5][0] in closes and last is not None and last[5][0] in closes, (closes, [b[5][0] for b in binds]))
    check("grant: the connection of the shortcuts ended before the process", any(t < end - 0.1 for _, t in rec["eof"]), (rec["eof"], end))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    ap.add_argument("--only", choices=["grant", "deny", "none"])
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"
    with tempfile.TemporaryDirectory() as d:
        for mode in ("grant", "deny", "none"):
            if a.only in (None, mode):
                scenario(exe, d, mode)
    print("screen_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
