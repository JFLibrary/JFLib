#!/usr/bin/env python3
"""keyburst.py -- bursts of synthetic keys beyond plat's 256-event ring on X11 and Wayland.

  python tools/keyburst.py [--exe build/linux/linuxtest] [--keys 1000] [--backend x11|wayland|both]

(Windows side) runs `linuxtest --test burst` in WSL behind a proxy between it and WSLg's X server
(this file's `x11proxy` mode) or compositor (tools/wlproxy/wlinject.py, scenario burst).  The
title "linuxtest burst" arms the proxy; at the client's next round trip (X11 GetInputFocus,
Wayland wl_display.sync) it writes N key presses + releases of the digit keys in ONE write ahead
of the request, so they arrive in one read while the backend waits for the reply; "linuxtest
burst 2" makes it write them at once while the client only waits for events.  linuxtest checks
that all 3 N events (E_KEY_DOWN, E_TEXT, E_KEY_UP per key) arrive in order.  The user's real
keys never reach the client (the proxies drop them).  Exit code 0 = every run passed.

  python3 tools/keyburst.py x11proxy LISTEN LOG TIMEOUT KEYS      (inside WSL)
"""
import os
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DISTRO = "Ubuntu-24.04"


# ---------------------------------------------------------------- the X11 proxy (inside WSL)

def x11proxy(listen, logf, timeout, keys):
    import select
    import socket
    import time
    log = open(logf, "w")

    def L(*a):
        print(*a, file=log, flush=True)

    def recvn(c, n):
        b = b""
        while len(b) < n:
            x = c.recv(n - len(b))
            if not x:
                raise SystemExit(0)
            b += x
        return b

    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind("\0" + listen[1:] if listen.startswith("@") else listen)
    srv.listen(1)
    print("READY", flush=True)
    srv.settimeout(timeout)
    try:
        c, _ = srv.accept()
    except socket.timeout:
        raise SystemExit(0)
    # connection setup: the client's request without its authorization (WSLg's server needs
    # none), the server's answer as it is
    hdr = recvn(c, 12)
    if hdr[0] != 0x6C:
        L("not a little-endian client")
        raise SystemExit(1)
    n, d = struct.unpack("<HH", hdr[6:10])
    recvn(c, ((n + 3) & ~3) + ((d + 3) & ~3))
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect("/tmp/.X11-unix/X0")
    s.sendall(hdr[:6] + b"\0\0\0\0" + hdr[10:12])
    head = recvn(s, 8)
    c.sendall(head + recvn(s, struct.unpack("<H", head[6:8])[0] * 4))
    st = dict(seq=0, armed=False, done2=False, win=0, root=0, time=1000000)
    parents = {}
    cbuf, sbuf = b"", b""

    def burst():
        # KeyPress / KeyRelease of the digit keys (keycodes 10 .. 19) to the titled window
        out = []
        for k in range(keys):
            kc = 10 + k % 10
            for typ in (2, 3):
                st["time"] += 1
                out.append(struct.pack("<BBHIIIIhhhhHBx", typ, kc, st["seq"] & 0xFFFF, st["time"], st["root"],
                                       st["win"], 0, 10, 10, 10, 10, 0, 1))
        c.sendall(b"".join(out))
        L("burst", keys, "keys to window", hex(st["win"]), "seq", st["seq"])

    def client_reqs():
        nonlocal cbuf
        while len(cbuf) >= 4:
            op, ln = cbuf[0], struct.unpack("<H", cbuf[2:4])[0] * 4
            if ln == 0:  # BIG-REQUESTS: the length follows
                if len(cbuf) < 8:
                    return
                ln = struct.unpack("<I", cbuf[4:8])[0] * 4
            if ln < 4 or len(cbuf) < ln:
                return
            req, cbuf = cbuf[:ln], cbuf[ln:]
            st["seq"] += 1
            if op == 1 and len(req) >= 12:  # CreateWindow: wid, parent
                wid, parent = struct.unpack("<II", req[4:12])
                parents[wid] = parent
            elif op == 18 and b"linuxtest burst" in req:  # ChangeProperty: the title
                st["win"] = struct.unpack("<I", req[4:8])[0]
                st["root"] = parents.get(st["win"], 0)
                if b"linuxtest burst 2" in req:
                    if not st["done2"]:
                        st["done2"] = True
                        burst()
                elif not st["done2"]:
                    st["armed"] = True
            elif op == 43 and st["armed"]:  # GetInputFocus: a round trip
                st["armed"] = False
                burst()
            s.sendall(req)

    def server_msgs():
        # events, replies and errors in order; the user's own key events stay out
        nonlocal sbuf
        out = []
        while len(sbuf) >= 32:
            typ = sbuf[0] & 0x7F
            ln = 32
            if typ == 1 or typ == 35:
                ln += struct.unpack("<I", sbuf[4:8])[0] * 4
            if len(sbuf) < ln:
                break
            m, sbuf = sbuf[:ln], sbuf[ln:]
            if typ in (2, 3):
                continue
            out.append(m)
        if out:
            c.sendall(b"".join(out))

    end = time.time() + timeout
    while time.time() < end:
        r, _, _ = select.select([c, s], [], [], 0.2)
        for a in r:
            try:
                b = a.recv(1 << 16)
                if a is c and b:
                    cbuf += b
                    client_reqs()
                elif b:
                    sbuf += b
                    server_msgs()
            except (ConnectionResetError, BrokenPipeError):
                b = b""
            if not b:
                L("closed")
                raise SystemExit(0)
    L("timeout")


# ---------------------------------------------------------------- the runs (Windows side)

def wsl_root():
    w = ROOT.replace("\\", "/")
    return "/mnt/" + w[0].lower() + w[2:]


def run(backend, exe, keys):
    os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
    plog = f"build/keyburst_{backend}_proxy.log"
    if backend == "x11":
        proxy_cmd = ["python3", "tools/keyburst.py", "x11proxy", "@/tmp/.X11-unix/X71", plog, "40", str(keys)]
        env = "jfgui_BACKEND=x11 DISPLAY=:71"
    else:
        proxy_cmd = ["python3", "tools/wlproxy/wlinject.py", "/mnt/wslg/runtime-dir/wayland-0",
                     "/run/user/1000/wayland-burst", plog, "40", "burst", "build/keyburst_wl.frame", str(keys)]
        env = "jfgui_BACKEND=wayland WAYLAND_DISPLAY=wayland-burst"
    proxy = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", wsl_root(), "-e"] + proxy_cmd,
                             stdout=subprocess.PIPE, text=True)
    proxy.stdout.readline()  # READY
    r = subprocess.run(["wsl.exe", "-d", DISTRO, "--cd", wsl_root(), "-e", "sh", "-c",
                        f"exec env {env} ./{exe} --test burst --keys {keys} 2>&1"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    try:
        proxy.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proxy.kill()
    print(f"--- {backend}: exit {r.returncode}")
    print(r.stdout.rstrip())
    try:
        print("    proxy: " + " | ".join(open(os.path.join(ROOT, plog), encoding="utf-8").read().split("\n")).strip(" |"))
    except OSError:
        print("    proxy: no log")
    return r.returncode == 0 and "PASS" in r.stdout and "FAIL" not in r.stdout


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "x11proxy":
        x11proxy(sys.argv[2], sys.argv[3], float(sys.argv[4]), int(sys.argv[5]))
        return 0
    import argparse
    ap = argparse.ArgumentParser(description="bursts of synthetic keys on X11 and Wayland (WSLg)")
    ap.add_argument("--exe", default="build/linux/linuxtest")
    ap.add_argument("--keys", type=int, default=1000)
    ap.add_argument("--backend", default="both", choices=["x11", "wayland", "both"])
    a = ap.parse_args()
    ok = True
    for b in (["x11", "wayland"] if a.backend == "both" else [a.backend]):
        ok &= run(b, a.exe, a.keys)
    print("keyburst: " + ("all passed" if ok else "FAILED"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
