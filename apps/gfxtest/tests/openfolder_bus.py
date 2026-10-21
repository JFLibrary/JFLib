#!/usr/bin/env python3
"""plat_open_folder on Linux against a fake session bus (U2).  Run inside WSL / Linux:

  python3 apps/gfxtest/tests/openfolder_bus.py build/u2/linux/gfxtest

A minimal D-Bus server (SASL EXTERNAL, NEGOTIATE_UNIX_FD, little-endian messages, SCM_RIGHTS)
plays org.freedesktop.portal.Desktop.  Cases: OpenDirectory with an O_PATH descriptor of the
path (checked through /proc/self/fd), the OpenURI fallback when the portal rejects the call or
the bus refuses descriptor passing (file:// URI, percent-encoded), Response codes 0 / 2, no
Response (the request is kept alive, 0 returned), a missing path, no bus.  Exit code 0 = passed.
"""
import os, socket, struct, subprocess, sys, tempfile, threading

EXE = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "build/u2/linux/gfxtest")
TMP = tempfile.mkdtemp(prefix="u2bus")
TARGET = os.path.join(TMP, "Ordner mit Ä")
os.mkdir(TARGET)
FILE = os.path.join(TARGET, "bild.png")
open(FILE, "wb").close()


def pad(b, n):
    return b + b"\0" * (-len(b) % n)


def s_val(s):
    e = s.encode()
    return struct.pack("<I", len(e)) + e + b"\0"


def field(code, sig, val):
    return bytes([code, 1, ord(sig), 0]) + (s_val(val) if sig in "so" else
                                            (bytes([len(val)]) + val.encode() + b"\0" if sig == "g"
                                             else struct.pack("<I", val)))


def message(mtype, serial, fields, body=b""):
    fb = b""
    for f in fields:
        fb = pad(fb, 8) + f
    hdr = b"l" + bytes([mtype, 0, 1]) + struct.pack("<III", len(body), serial, len(fb)) + fb
    return pad(hdr, 8) + body


def parse(buf):
    """-> (msg dict, rest) or (None, buf)"""
    if len(buf) < 16:
        return None, buf
    blen, serial, flen = struct.unpack_from("<III", buf, 4)
    hend = 16 + flen
    total = hend + (-hend % 8) + blen
    if len(buf) < total:
        return None, buf
    m = {"type": buf[1], "serial": serial, "fields": {}}
    p = 16
    while p < hend:
        p += -p % 8
        code, slen = buf[p], buf[p + 1]
        sig = chr(buf[p + 2])
        p += 3 + slen
        if sig in "so":
            p += -p % 4
            n = struct.unpack_from("<I", buf, p)[0]
            m["fields"][code] = buf[p + 4:p + 4 + n].decode()
            p += 5 + n
        elif sig == "g":
            n = buf[p]
            m["fields"][code] = buf[p + 1:p + 1 + n].decode()
            p += 2 + n
        else:
            p += -p % 4
            m["fields"][code] = struct.unpack_from("<I", buf, p)[0]
            p += 4
    m["body"] = buf[hend + (-hend % 8):total]
    return m, buf[total:]


def body_strings(body, sig):
    """values of the leading s / o / h arguments"""
    out, p = [], 0
    for c in sig:
        if c in "so":
            p += -p % 4
            n = struct.unpack_from("<I", body, p)[0]
            out.append(body[p + 4:p + 4 + n].decode())
            p += 5 + n
        elif c == "h":
            p += -p % 4
            out.append(struct.unpack_from("<I", body, p)[0])
            p += 4
        else:
            break
    return out


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
    c.settimeout(10)
    f = b""
    # ---- SASL
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
    fds, sser = [], 100
    handle = "/org/freedesktop/portal/desktop/request/1_42/x"
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
        reply = lambda sig, body: message(2, sser, [field(5, "u", m["serial"]), field(6, "s", ":1.42"),
                                                    field(7, "s", "org.freedesktop.DBus")] +
                                          ([field(8, "g", sig)] if sig else []), body)
        sser += 1
        if member == "Hello":
            c.sendall(reply("s", s_val(":1.42")))
        elif member == "AddMatch":
            log.append("match " + body_strings(m["body"], "s")[0])
            c.sendall(reply("", b""))
        elif member in ("OpenDirectory", "OpenURI"):
            sig = m["fields"].get(8, "")
            args = body_strings(m["body"], sig)
            if member == "OpenDirectory":
                nfd = m["fields"].get(9, 0)
                link = os.readlink(f"/proc/self/fd/{fds[0]}") if fds else "?"
                log.append(f"sig {sig} unix_fds {nfd} fds {len(fds)} -> {link}")
                for d in fds:
                    os.close(d)
                fds = []
                if mode == "reject":
                    c.sendall(message(3, sser, [field(4, "s", "org.freedesktop.DBus.Error.UnknownMethod"),
                                                field(5, "u", m["serial"]), field(6, "s", ":1.42")]))
                    continue
            else:
                log.append(f"sig {sig} uri {args[1]}")
            c.sendall(reply("o", s_val(handle)))
            if mode == "silent":
                continue
            code = 2 if mode == "fail" else 0
            body = struct.pack("<I", code) + struct.pack("<I", 0) + b"\0" * 4   # u, empty a{sv}
            c.sendall(message(4, sser + 50, [field(1, "o", handle),
                                             field(2, "s", "org.freedesktop.portal.Request"),
                                             field(3, "s", "Response"), field(6, "s", ":1.42"),
                                             field(8, "g", "ua{sv}")], body))


RUNS = [0]


def run(mode, path, bus=True):
    RUNS[0] += 1
    sock = os.path.join(TMP, f"bus-{RUNS[0]}-{mode}")
    log = []
    env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}" if bus else "unix:path=/nonexistent")
    env.pop("jfgui_OPEN_DRYRUN", None)
    t = None
    if bus:
        t = threading.Thread(target=serve, args=(sock, mode, log), daemon=True)
        t.start()
        while not os.path.exists(sock):
            pass
    p = subprocess.run([EXE, "--shot", os.path.join(TMP, "x.png"), "--open", path], env=env,
                       capture_output=True, text=True, timeout=60)
    if t:
        t.join(5)
    return p.returncode, log


fails = 0


def check(name, ok, info):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1


uri = "file://" + TARGET.replace(" ", "%20").replace("Ä", "%C3%84")
rc, log = run("ok", TARGET)
check("OpenDirectory with an O_PATH descriptor of the folder",
      rc == 0 and f"sig sha{{sv}} unix_fds 1 fds 1 -> {TARGET}" in log and "NEGOTIATE_UNIX_FD" in log
      and any(l.startswith("match ") and "request/1_42/jfgui" in l for l in log), (rc, log))
rc, log = run("ok", FILE)
check("a file: its descriptor", rc == 0 and f"sig sha{{sv}} unix_fds 1 fds 1 -> {FILE}" in log, (rc, log))
rc, log = run("reject", TARGET)
check("rejected OpenDirectory -> OpenURI(file://)", rc == 0 and f"sig ssa{{sv}} uri {uri}" in log, (rc, log))
rc, log = run("nofd", TARGET)
check("no descriptor passing -> OpenURI only", rc == 0 and "OpenDirectory" not in log
      and f"sig ssa{{sv}} uri {uri}" in log, (rc, log))
rc, log = run("fail", TARGET)
check("Response 2 -> -1", rc == 1, (rc, log))
rc, log = run("silent", TARGET)
check("no Response: request kept, 0", rc == 0 and "OpenDirectory" in log, (rc, log))
rc, log = run("ok", os.path.join(TMP, "gibt es nicht"))
check("missing path -> -1 without a bus connection", rc == 1 and log == [], (rc, log))
rc, log = run("ok", TARGET, bus=False)
check("no bus -> -1", rc == 1, (rc, log))
print("openfolder_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
sys.exit(1 if fails else 0)
