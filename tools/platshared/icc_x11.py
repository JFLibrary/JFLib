#!/usr/bin/env python3
"""icc_x11.py -- plat::monitor_icc on X11 (lx_icc.s: the root window's _ICC_PROFILE), agent
plat-shared R4.  Run inside WSL (WSLg's X server) from the repository root:

  python3 tools/platshared/icc_x11.py [--rs build]

A small raw X11 client sets _ICC_PROFILE on the root window for each run of plattest --test
iccx11 (an unmapped window of its own, never shown) and puts the property back as it was
afterwards: a valid profile (its length and hash must come back), an invalid one ('acsp'
missing), one above 1 MB (more than one GetProperty chunk), none.  No assembly run: the
assembly's only callers need a real window (PaintCraft's colour management).  Exit 0 = passed.
"""
import argparse, os, socket, struct, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1


class X:
    def __init__(self):
        n = os.environ.get("DISPLAY", ":0").split(":")[-1].split(".")[0]
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.connect(f"/tmp/.X11-unix/X{n}")
        self.s.sendall(struct.pack("<BxHHHHxx", 0x6C, 11, 0, 0, 0))
        hdr = self.read(8)
        if hdr[0] != 1:
            raise RuntimeError("X11 setup refused")
        body = self.read(struct.unpack_from("<H", hdr, 6)[0] * 4)
        vlen, = struct.unpack_from("<H", body, 16)
        nfmt = body[21]
        o = 32 + (vlen + 3) // 4 * 4 + nfmt * 8
        self.root, = struct.unpack_from("<I", body, o)

    def read(self, n):
        b = b""
        while len(b) < n:
            d = self.s.recv(n - len(b))
            if not d:
                raise RuntimeError("X11 connection closed")
            b += d
        return b

    def reply(self):
        while True:
            p = self.read(32)
            if p[0] == 0:
                raise RuntimeError(f"X error {p[1]}")
            if p[0] == 1:
                return p + self.read(struct.unpack_from("<I", p, 4)[0] * 4)

    def req(self, op, data1, body):
        body += b"\0" * (-len(body) % 4)
        self.s.sendall(struct.pack("<BBH", op, data1, 1 + len(body) // 4) + body)

    def sync(self):
        self.req(43, 0, b"")
        self.reply()

    def atom(self, name):
        n = name.encode()
        self.req(16, 0, struct.pack("<HH", len(n), 0) + n)
        return struct.unpack_from("<I", self.reply(), 8)[0]

    def get(self, atom):
        self.req(20, 0, struct.pack("<IIIII", self.root, atom, 0, 0, 1 << 20))
        r = self.reply()
        fmt, typ, after, n = r[1], *struct.unpack_from("<III", r, 8)
        if typ == 0:
            return None
        return (typ, fmt, r[32:32 + n * (fmt // 8)], after)

    def set(self, atom, typ, fmt, data):
        """Replace, then Append in chunks (no BIG-REQUESTS needed)"""
        step = 200000 // 4 * 4
        for k in range(0, max(len(data), 1), step):
            part = data[k:k + step]
            n = len(part) // (fmt // 8)
            self.req(18, 0 if k == 0 else 2, struct.pack("<IIIB3xI", self.root, atom, typ, fmt, n) + part)
        self.sync()

    def delete(self, atom):
        self.req(19, 0, struct.pack("<II", self.root, atom))
        self.sync()


def profile(size):
    """a valid ICC header (version 4, 'mntr', 'RGB ', 'XYZ ', 'acsp') with two tags"""
    b = bytearray(size)
    struct.pack_into(">I", b, 0, size)
    b[8] = 4
    b[12:16], b[16:20], b[20:24], b[36:40] = b"mntr", b"RGB ", b"XYZ ", b"acsp"
    struct.pack_into(">I", b, 128, 2)
    struct.pack_into(">4sII", b, 132, b"desc", 156, 40)
    struct.pack_into(">4sII", b, 144, b"wtpt", 196, 20)
    for i in range(200, size):
        b[i] = (i * 7) & 255
    return bytes(b)


def fnv(b):
    h = 0xCBF29CE484222325
    for c in b:
        h = ((h ^ c) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def run(rs):
    env = dict(os.environ, jfgui_BACKEND="x11")
    p = subprocess.run([f"./{rs}/linux/plattest", "--test", "iccx11"], cwd=ROOT, env=env, capture_output=True, text=True,
                       timeout=60)
    line = [l.strip() for l in p.stdout.splitlines() if l.strip().startswith("icc: window 0:")]
    return p.returncode, (line[0] if line else ""), p.stdout + p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    a = ap.parse_args()
    x = X()
    atom = x.atom("_ICC_PROFILE")
    old = x.get(atom)
    try:
        good = profile(2000)
        x.set(atom, 6, 8, good)  # CARDINAL, format 8
        rc, line, out = run(a.rs)
        check("iccx11: a valid _ICC_PROFILE comes back (length, hash)",
              rc == 0 and line == f"icc: window 0: {len(good)} bytes, fnv {fnv(good):016x}", (line, out[-400:]))
        bad = bytearray(good)
        bad[36:40] = b"xxxx"
        x.set(atom, 6, 8, bytes(bad))
        rc, line, out = run(a.rs)
        check("iccx11: an invalid profile ('acsp' missing) -> none", rc == 0 and line == "icc: window 0: none", (line, out[-400:]))
        big = profile(1100000)
        x.set(atom, 6, 8, big)
        rc, line, out = run(a.rs)
        check("iccx11: a profile above 1 MB -> none", rc == 0 and line == "icc: window 0: none", (line, out[-400:]))
        x.delete(atom)
        rc, line, out = run(a.rs)
        check("iccx11: no _ICC_PROFILE -> none", rc == 0 and line == "icc: window 0: none", (line, out[-400:]))
    finally:
        if old is None:
            x.delete(atom)
        else:
            x.set(atom, old[0], old[1], old[2])
        check("iccx11: the root window's _ICC_PROFILE is as before", x.get(atom) == old)
    print("icc_x11:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
