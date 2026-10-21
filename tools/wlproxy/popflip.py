#!/usr/bin/env python3
"""popflip.py -- (ROADMAP 6, inside WSL) a compositor that moves popups.

Weston (WSLg) places an xdg_popup where its positioner says, even off the screen; GNOME and KDE
flip and slide popups to keep them on the screen and tell the client where in
xdg_popup.configure.  This proxy between one client and WSLg's compositor plays such a
compositor: it rewrites the position of every xdg_popup.configure to --shift px higher (as a
compositor that slid the popup up).  The gallery (--popup-edges-open, 600 x 360,
jfgui_WLDEBUG=1) opens its combo box's list, which does not fit into the window and becomes an
xdg_popup; the app must lay the list out where the configure put it: its trace shows the
content at the configured place ("[wl] popup K: content at X, Y (natural NX, NY)"), and the
popup is kept (one xdg_surface.get_popup), so the pointer offsets follow the content.

  python3 tools/wlproxy/popflip.py [--exe build/linux/gallery] [--shift 120]

Run from the repository root inside WSL (Windows: wsl.exe -d Ubuntu-24.04 --cd <repo> -e
python3 tools/wlproxy/popflip.py).  Exit code 0 = passed.
"""
import argparse
import array
import os
import re
import select
import socket
import struct
import subprocess
import sys
import time

fails = []


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not ok:
        fails.append(name)


def recv(sock):
    fds = array.array("i")
    data, anc, _, _ = sock.recvmsg(65536, socket.CMSG_SPACE(64 * 4))
    for lvl, typ, d in anc:
        if lvl == socket.SOL_SOCKET and typ == socket.SCM_RIGHTS:
            fds.frombytes(d[:len(d) - len(d) % 4])
    return data, list(fds)


def send(sock, data, fds):
    anc = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds))] if fds else []
    sock.sendmsg([data], anc)
    for f in fds:
        os.close(f)


def messages(buf):
    """complete messages at the start of buf -> ([(object, opcode, body offset, size)], rest offset)"""
    out, o = [], 0
    while len(buf) - o >= 8:
        obj, hdr = struct.unpack_from("<II", buf, o)
        size = hdr >> 16
        if size < 8 or len(buf) - o < size:
            break
        out.append((obj, hdr & 0xFFFF, o, size))
        o += size
    return out, o


def proxy(lsock, real, shift, state, stop):
    c, _ = lsock.accept()
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(real)
    cbuf, sbuf = b"", bytearray()
    sfds = []
    while not stop():
        r, _, _ = select.select([c, s], [], [], 0.2)
        if c in r:
            data, fds = recv(c)
            if not data and not fds:
                break
            cbuf += data
            # client requests: who is the wm_base, which objects are xdg_surfaces and popups
            msgs, used = messages(cbuf)
            for obj, op, o, size in msgs:
                if obj == 1 and op == 1:                                    # wl_display.get_registry
                    state["registry"] = struct.unpack_from("<I", cbuf, o + 8)[0]
                elif obj == state.get("registry") and op == 0:              # wl_registry.bind
                    n = struct.unpack_from("<I", cbuf, o + 12)[0]
                    iface = cbuf[o + 16:o + 16 + n - 1].decode()
                    new = struct.unpack_from("<I", cbuf, o + 16 + ((n + 3) & ~3) + 4)[0]
                    if iface == "xdg_wm_base":
                        state["wm"] = new
                elif obj == state.get("wm") and op == 2:                    # get_xdg_surface
                    state["xsurfs"].add(struct.unpack_from("<I", cbuf, o + 8)[0])
                elif obj in state["xsurfs"] and op == 2:                    # xdg_surface.get_popup
                    state["popups"].add(struct.unpack_from("<I", cbuf, o + 8)[0])
                    state["get_popup"] += 1
            cbuf = cbuf[used:]
            send(s, data, fds)
        if s in r:
            data, fds = recv(s)
            if not data and not fds:
                break
            sbuf += data
            sfds += fds
            msgs, used = messages(sbuf)
            for obj, op, o, size in msgs:
                if obj in state["popups"] and op == 0 and size >= 24:        # xdg_popup.configure
                    x, y, w, h = struct.unpack_from("<iiii", sbuf, o + 8)
                    struct.pack_into("<i", sbuf, o + 12, y - shift)
                    state["configures"].append((x, y, w, h))
            if used:
                send(c, bytes(sbuf[:used]), sfds)
                sbuf = sbuf[used:]
                sfds = []
    c.close()
    s.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default="build/linux/gallery")
    ap.add_argument("--shift", type=int, default=120)
    a = ap.parse_args()
    rt = os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid())
    real = os.path.join(rt, os.environ.get("WAYLAND_DISPLAY", "wayland-0"))
    if not os.path.exists(real):
        real = "/mnt/wslg/runtime-dir/wayland-0"
    name = "wayland-popflip-%d" % os.getpid()
    path = os.path.join(rt, name)
    lsock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    lsock.bind(path)
    lsock.listen(1)
    state = {"xsurfs": set(), "popups": set(), "get_popup": 0, "configures": []}
    done = []
    import threading
    t = threading.Thread(target=proxy, args=(lsock, real, a.shift, state, lambda: bool(done)), daemon=True)
    t.start()
    env = dict(os.environ, WAYLAND_DISPLAY=name, jfgui_BACKEND="wayland", jfgui_WLDEBUG="1", jfgui_CSD="1")
    exe = a.exe if a.exe.startswith("/") else "./" + a.exe
    p = subprocess.run([exe, "--popup-edges-open", "--size", "600x360", "--anim", "off", "--theme", "light",
                        "--auto-close", "5000"], env=env, capture_output=True, text=True, errors="replace", timeout=60)
    done.append(1)
    t.join(3)
    os.unlink(path)
    log = p.stdout + p.stderr
    check(p.returncode == 0, "the app ran", str(p.returncode))
    check(state["get_popup"] >= 1 and state["configures"], "the combo box's list became an xdg_popup and was configured",
          f"{state['get_popup']} get_popup, configures {state['configures']}")
    if not state["configures"]:
        return 1
    x, y, w, h = state["configures"][0]
    seen = re.findall(r"xdg_popup#\d+\.configure\((-?\d+), (-?\d+), (\d+), (\d+)\)", log)
    check(seen and int(seen[0][1]) == y - a.shift, "the app got the moved position", f"{seen[:1]} vs {(x, y - a.shift)}")
    moved = re.findall(r"\[wl\] popup \d+: content at (-?\d+), (-?\d+) \(natural (-?\d+), (-?\d+)\)", log)
    scale = 1
    m = re.search(r"set_buffer_scale\((\d+)\)", log)
    if m:
        scale = int(m.group(1))
    check(any(int(mx) == x * scale and int(my) == (y - a.shift) * scale for mx, my, _, _ in moved),
          "the app laid the list out where the compositor put it", f"{moved[-3:]} want {x * scale}, {(y - a.shift) * scale}")
    check(state["get_popup"] == 1, "the popup was kept (one xdg_surface.get_popup)", str(state["get_popup"]))
    if fails:
        tail = [ln for ln in log.splitlines() if "popup" in ln][-20:]
        print("\n".join(tail))
    print("popflip:", "all passed" if not fails else f"{len(fails)} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
