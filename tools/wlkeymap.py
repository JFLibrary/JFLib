#!/usr/bin/env python3
"""Dump the compositor's XKB keymap and the Wayland globals (Linux / WSLg).

A tiny pure-Python Wayland client used to test the assembly XKB parser
offline:  it binds wl_seat, asks for a wl_keyboard and saves the keymap text
the compositor sends (the same bytes plat/linux/xkb.s parses).

  python3 tools/wlkeymap.py --out build/keymap.xkb      (run inside WSL)
"""
import argparse
import array
import mmap
import os
import socket
import struct
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="build/keymap.xkb")
    a = ap.parse_args()
    disp = os.environ.get("WAYLAND_DISPLAY", "wayland-0")
    path = disp if disp.startswith("/") else os.path.join(
        os.environ.get("XDG_RUNTIME_DIR", "/run/user/%d" % os.getuid()), disp)
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    next_id = [2]

    def new_id():
        i = next_id[0]
        next_id[0] += 1
        return i

    def send(obj, op, payload=b"", fds=()):
        msg = struct.pack("<II", obj, ((8 + len(payload)) << 16) | op) + payload
        anc = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds))] if fds else []
        s.sendmsg([msg], anc)

    def wstr(t):
        b = t.encode() + b"\0"
        return struct.pack("<I", len(b)) + b + b"\0" * ((4 - len(b) % 4) % 4)

    registry = new_id()
    send(1, 1, struct.pack("<I", registry))
    sync = new_id()
    send(1, 0, struct.pack("<I", sync))
    buf, fdq = b"", []
    seat = keyboard = None
    done_sync = False
    keymap = None
    while keymap is None:
        data, anc, _, _ = s.recvmsg(65536, socket.CMSG_SPACE(64 * 4))
        if not data:
            break
        for lvl, typ, d in anc:
            if lvl == socket.SOL_SOCKET and typ == socket.SCM_RIGHTS:
                fdq += list(array.array("i", d[: len(d) - len(d) % 4]))
        buf += data
        while len(buf) >= 8:
            obj, w = struct.unpack_from("<II", buf)
            size, op = w >> 16, w & 0xFFFF
            if len(buf) < size:
                break
            body, buf = buf[8:size], buf[size:]
            if obj == registry and op == 0:
                name, ln = struct.unpack_from("<II", body)
                iface = body[8:8 + ln - 1].decode()
                ver = struct.unpack_from("<I", body, 8 + ((ln + 3) & ~3))[0]
                print(f"global {name:3d} {iface} v{ver}")
                if iface == "wl_seat" and seat is None:
                    seat = new_id()
                    send(registry, 0, struct.pack("<I", name) + wstr(iface) + struct.pack("<II", min(ver, 7), seat))
            elif obj == sync and op == 0:
                done_sync = True
                if seat:
                    keyboard = new_id()
                    send(seat, 1, struct.pack("<I", keyboard))
            elif obj == keyboard and op == 0:
                fmt, size_km = struct.unpack_from("<II", body)
                fd = fdq.pop(0)
                with mmap.mmap(fd, size_km, mmap.MAP_PRIVATE, mmap.PROT_READ) as m:
                    keymap = m[:].rstrip(b"\0")
                os.close(fd)
                print(f"keymap format {fmt}, {size_km} bytes")
            elif obj == keyboard and op == 5:
                print("repeat_info rate %d delay %d" % struct.unpack_from("<ii", body))
            elif obj == 1 and op == 0:
                print("protocol error", body)
                return 1
    if keymap is None:
        print("no keymap received")
        return 1
    with open(a.out, "wb") as f:
        f.write(keymap)
    print("wrote", a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
