#!/usr/bin/env python3
"""plat_print on Linux against a fake session bus (PR1).  Run inside WSL / Linux:

  python3 tools/print_portal_bus.py ./build/pr1/linux/PaintCraft build/pr1

A minimal D-Bus server (SASL EXTERNAL, NEGOTIATE_UNIX_FD, little-endian, SCM_RIGHTS) plays
org.freedesktop.portal.Print: PreparePrint (arguments checked: title, settings / page_setup
suggestions, options) answers with a page setup (US Letter, landscape, 6.35 mm margins) and a
token; Print must bring that token and a descriptor whose content is checked like
print_check.py checks jfgui_PRINT_TO files (layout on the portal's page, pixels over white).
Cases: ok, cancelled dialog, no portal (error reply), Print failing (Response 2), a bus
without descriptor passing, headless without jfgui_PRINT_PORTAL (no bus contact), no bus.
PaintCraft runs headless with jfgui_PRINT_PORTAL=1.  Exit code 0 = passed.
"""
import os, socket, struct, subprocess, sys, tempfile, threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import print_check as pc  # noqa: E402

EXE = sys.argv[1] if len(sys.argv) > 1 else "./build/pr1/linux/PaintCraft"
OUT = (sys.argv[2] if len(sys.argv) > 2 else "build/pr1").rstrip("/")
WORK = f"{OUT}/print_check"
TMP = tempfile.mkdtemp(prefix="pr1bus")
TOKEN = 4242
MM = 72 / 25.4


def pad(b, n):
    return b + b"\0" * (-len(b) % n)


# ------------------------------------------------------------------ marshalling
class W:
    def __init__(self):
        self.b = bytearray()

    def align(self, n):
        self.b += b"\0" * (-len(self.b) % n)

    def val(self, sig, v):
        c = sig[0]
        if c in "so":
            e = v.encode()
            self.align(4)
            self.b += struct.pack("<I", len(e)) + e + b"\0"
        elif c == "g":
            e = v.encode()
            self.b += bytes([len(e)]) + e + b"\0"
        elif c in "ubh":
            self.align(4)
            self.b += struct.pack("<I", int(v))
        elif c == "d":
            self.align(8)
            self.b += struct.pack("<d", v)
        elif c == "v":
            vs, vv = v
            self.val("g", vs)
            self.val(vs, vv)
        elif sig.startswith("a{sv}"):
            self.align(4)
            at = len(self.b)
            self.b += b"\0\0\0\0"
            self.align(8)
            start = len(self.b)
            for k, (vs, vv) in v.items():
                self.align(8)
                self.val("s", k)
                self.val("v", (vs, vv))
            struct.pack_into("<I", self.b, at, len(self.b) - start)
        else:
            raise ValueError(sig)


class R:
    def __init__(self, b):
        self.b, self.p = b, 0

    def align(self, n):
        self.p += -self.p % n

    def one(self, sig):
        """-> (value, rest of sig)"""
        c = sig[0]
        if c in "so":
            self.align(4)
            n = struct.unpack_from("<I", self.b, self.p)[0]
            v = self.b[self.p + 4:self.p + 4 + n].decode()
            self.p += 5 + n
            return v, sig[1:]
        if c == "g":
            n = self.b[self.p]
            v = self.b[self.p + 1:self.p + 1 + n].decode()
            self.p += 2 + n
            return v, sig[1:]
        if c in "ubhi":
            self.align(4)
            v = struct.unpack_from("<I", self.b, self.p)[0]
            self.p += 4
            return v, sig[1:]
        if c == "d":
            self.align(8)
            v = struct.unpack_from("<d", self.b, self.p)[0]
            self.p += 8
            return v, sig[1:]
        if c == "v":
            vs, _ = self.one("g")
            v, _ = self.one(vs)
            return v, sig[1:]
        if sig.startswith("a{sv}"):
            self.align(4)
            n = struct.unpack_from("<I", self.b, self.p)[0]
            self.p += 4
            self.align(8)
            end, d = self.p + n, {}
            while self.p < end:
                self.align(8)
                k, _ = self.one("s")
                d[k], _ = self.one("v")
            return d, sig[5:]
        raise ValueError(sig)

    def all(self, sig):
        out = []
        while sig:
            v, sig = self.one(sig)
            out.append(v)
        return out


def field(code, sig, val):
    w = W()
    w.b += bytes([code, 1, ord(sig), 0])
    w.val(sig, val)
    return bytes(w.b)


def message(mtype, serial, fields, body=b""):
    fb = b""
    for f in fields:
        fb = pad(fb, 8) + f
    hdr = b"l" + bytes([mtype, 0, 1]) + struct.pack("<III", len(body), serial, len(fb)) + fb
    return pad(hdr, 8) + body


def parse(buf):
    if len(buf) < 16:
        return None, buf
    blen, serial, flen = struct.unpack_from("<III", buf, 4)
    hend = 16 + flen
    total = hend + (-hend % 8) + blen
    if len(buf) < total:
        return None, buf
    m = {"type": buf[1], "serial": serial, "fields": {}}
    r = R(buf)
    r.p = 16
    while r.p < hend:
        r.align(8)
        code = buf[r.p]
        r.p += 1
        sig, _ = r.one("g")
        m["fields"][code], _ = r.one(sig)
    m["body"] = buf[hend + (-hend % 8):total]
    return m, buf[total:]


# ------------------------------------------------------------------ the fake bus
def serve(path, mode, log):
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(1)
    srv.settimeout(30)
    try:
        c, _ = srv.accept()
    except socket.timeout:
        log.append("no connection")
        return
    c.settimeout(20)
    f = b""
    while True:
        while b"\r\n" not in f:
            d = c.recv(4096)
            if not d:
                return
            f += d
        line, f = f.split(b"\r\n", 1)
        line = line.lstrip(b"\0")
        log.append(line.decode())
        if line.startswith(b"AUTH"):
            c.sendall(b"OK 0123456789abcdef0123456789abcdef\r\n")
        elif line.startswith(b"NEGOTIATE_UNIX_FD"):
            c.sendall(b"ERROR\r\n" if mode == "nofd" else b"AGREE_UNIX_FD\r\n")
        elif line.startswith(b"BEGIN"):
            break
    fds, sser, nreq = [], 100, 0
    while True:
        m, f = parse(f)
        if m is None:
            try:
                d, got, _, _ = socket.recv_fds(c, 65536, 4)
            except (socket.timeout, OSError):
                return
            if not d:
                return
            fds += got
            f += d
            continue
        member = m["fields"].get(3, "")
        log.append(member)
        sser += 1

        def reply(sig, body):
            return message(2, sser, [field(5, "u", m["serial"]), field(6, "s", ":1.42")] +
                           ([field(8, "g", sig)] if sig else []), body)

        def response(handle, code, results):
            w = W()
            w.val("u", code)
            w.val("a{sv}", results)
            return message(4, sser + 50, [field(1, "o", handle), field(2, "s", "org.freedesktop.portal.Request"),
                                          field(3, "s", "Response"), field(6, "s", ":1.42"),
                                          field(8, "g", "ua{sv}")], bytes(w.b))
        if member == "Hello":
            w = W()
            w.val("s", ":1.42")
            c.sendall(reply("s", bytes(w.b)))
        elif member == "AddMatch":
            log.append("match " + R(m["body"]).all("s")[0])
            c.sendall(reply("", b""))
        elif member in ("PreparePrint", "Print"):
            sig = m["fields"].get(8, "")
            log.append(f"{member} sig {sig} iface {m['fields'].get(2)} path {m['fields'].get(1)}")
            if mode == "noportal":
                c.sendall(message(3, sser, [field(4, "s", "org.freedesktop.DBus.Error.UnknownMethod"),
                                            field(5, "u", m["serial"]), field(6, "s", ":1.42")]))
                continue
            args = R(m["body"]).all(sig)
            log.append(("args", member, args))
            nreq += 1
            handle = f"/org/freedesktop/portal/desktop/request/1_42/r{nreq}"
            w = W()
            w.val("o", handle)
            c.sendall(reply("o", bytes(w.b)))
            if member == "PreparePrint":
                if mode == "cancel":
                    c.sendall(response(handle, 1, {}))
                    continue
                setup = {"PPDName": ("s", "Letter"), "Name": ("s", "na_letter"),
                         "DisplayName": ("s", "US Letter"), "Width": ("d", 215.9), "Height": ("d", 279.4),
                         "MarginTop": ("d", 6.35), "MarginBottom": ("d", 6.35), "MarginLeft": ("d", 6.35),
                         "MarginRight": ("d", 6.35), "Orientation": ("s", "landscape")}
                c.sendall(response(handle, 0, {"settings": ("a{sv}", {"n-copies": ("s", "2")}),
                                               "page-setup": ("a{sv}", setup), "token": ("u", TOKEN)}))
            else:
                nfd = m["fields"].get(9, 0)
                data = b""
                if fds:
                    os.lseek(fds[0], 0, os.SEEK_SET)
                    while True:
                        chunk = os.read(fds[0], 1 << 20)
                        if not chunk:
                            break
                        data += chunk
                log.append(("fd", nfd, len(fds), data))
                for d in fds:
                    os.close(d)
                fds = []
                c.sendall(response(handle, 2 if mode == "printfail" else 0, {}))


RUNS = [0]


def run(mode, bus=True, portal_env=True, error=None):
    RUNS[0] += 1
    sock = os.path.join(TMP, f"bus-{RUNS[0]}-{mode}")
    log = []
    env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}" if bus else "unix:path=/nonexistent")
    for k in ("jfgui_PRINT_TO", "jfgui_PRINT_PORTAL", "jfgui_PRINT_SMASK"):
        env.pop(k, None)
    if portal_env:
        env["jfgui_PRINT_PORTAL"] = "1"
    t = None
    if bus:
        t = threading.Thread(target=serve, args=(sock, mode, log), daemon=True)
        t.start()
        while not os.path.exists(sock):
            pass
    scr = f"{WORK}/bus_{mode}.uis"
    pc.script(scr, [], None, error)
    p = subprocess.run([EXE, "--config", f"{OUT}/cfg", "--open", f"{WORK}/druck_test.png", "--script", scr],
                       env=env, capture_output=True, text=True, timeout=120)
    if t:
        t.join(3 if not portal_env else 25)
    return p.returncode, log, p.stdout + p.stderr


def main():
    px = pc.make_pixels()
    os.makedirs(WORK, exist_ok=True)
    pc.write_png(f"{WORK}/druck_test.png", px)
    ck = pc.check
    rc, log, out = run("ok")
    prep = [a for a in log if isinstance(a, tuple) and a[0] == "args" and a[1] == "PreparePrint"]
    ok = bool(prep)
    if ok:
        parent, title, settings, setup, opts = prep[0][2]
        ok = (title == "druck_test.png" and settings.get("orientation") == "portrait"
              and setup.get("Name") == "iso_a4" and abs(setup.get("Width", 0) - 210.0) < 1e-9
              and abs(setup.get("Height", 0) - 297.0) < 1e-9 and setup.get("Orientation") == "portrait"
              and opts.get("modal") == 1 and str(opts.get("handle_token", "")).startswith("jfgui"))
    ck("PreparePrint(parent, title, settings, page_setup, options): A4 portrait suggested",
       rc == 0 and ok and "NEGOTIATE_UNIX_FD" in log and "PreparePrint sig ssa{sv}a{sv}a{sv} iface "
       "org.freedesktop.portal.Print path /org/freedesktop/portal/desktop" in log, (rc, log, out[-400:]))
    pr = [a for a in log if isinstance(a, tuple) and a[0] == "args" and a[1] == "Print"]
    fd = [a for a in log if isinstance(a, tuple) and a[0] == "fd"]
    ok = bool(pr) and bool(fd)
    if ok:
        parent, title, h, opts = pr[0][2]
        ok = title == "druck_test.png" and h == 0 and opts.get("token") == TOKEN and fd[0][1] == 1 and fd[0][2] == 1
    ck("Print(parent, title, h, options): the token and one descriptor", ok, log)
    if fd and fd[0][3]:
        lay = pc.layout(0, 0, 0, 100, 1, 5, page=(279.4 * MM, 215.9 * MM), insets=(6.35 * MM,) * 4)
        pc.check_ours("portal PDF (Letter quer from the page setup)", fd[0][3], px, lay)
    rc, log, out = run("cancel")
    ck("cancelled dialog: no Print, no error", rc == 0 and "PreparePrint" in log and "Print" not in log,
       (rc, log, out[-400:]))
    rc, log, out = run("noportal", error=pc.MSG_UNSUPPORTED)
    ck("no Print portal (error reply) -> 'kein Drucker'", rc == 0 and "PreparePrint" in log, (rc, log, out[-400:]))
    rc, log, out = run("printfail", error=pc.MSG_FAIL)
    ck("Print Response 2 -> error", rc == 0 and "Print" in log, (rc, log, out[-400:]))
    rc, log, out = run("nofd", error=pc.MSG_UNSUPPORTED)
    ck("bus without descriptor passing -> 'kein Drucker', no PreparePrint",
       rc == 0 and "PreparePrint" not in log, (rc, log, out[-400:]))
    rc, log, out = run("ok", portal_env=False, error=pc.MSG_UNSUPPORTED)
    ck("headless without jfgui_PRINT_PORTAL: no bus contact", rc == 0 and not any(isinstance(l, str) and l.startswith("AUTH") for l in log),
       (rc, log, out[-400:]))
    rc, log, out = run("ok", bus=False, error=pc.MSG_UNSUPPORTED)
    ck("no bus -> 'kein Drucker'", rc == 0, (rc, log, out[-400:]))
    print("print_portal_bus:", "all passed" if pc.fails == 0 else f"{pc.fails} FAILED")
    return 1 if pc.fails else 0


if __name__ == "__main__":
    sys.exit(main())
