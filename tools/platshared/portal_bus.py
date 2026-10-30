#!/usr/bin/env python3
"""portal_bus.py -- plat's xdg-desktop-portal calls (lib/plat/linux_portal.rs) against a fake
session bus, the assembly next to the Rust build wherever the assembly can run them headless
(agent plat-shared, R4).  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/portal_bus.py [--ref build/rwref] [--rs build] [--only open,url,filedlg,print]

The fake bus (SASL EXTERNAL, NEGOTIATE_UNIX_FD, little-endian messages, SCM_RIGHTS) plays
org.freedesktop.portal.Desktop.  Every message a client sends is recorded raw.

  open     the cases of apps/gfxtest/tests/openfolder_bus.py (OpenDirectory with an O_PATH
           descriptor, OpenURI fallback when rejected / without descriptor passing, Response 2,
           no Response, a missing path, no bus): the assembly's gfxtest --open and plattest
           --test portal --mode open; SASL lines, every message byte for byte and the exit code
           must be equal
  url      OpenURI(parent, url, {handle_token}) of plattest --mode url (the assembly has no
           headless caller of plat_open_url): arguments checked, the body re-marshalled from
           its parsed values must equal the bytes received
  filedlg  FileChooser OpenFile / SaveFile of plattest --mode filedlg (ui_filedlg never calls
           the system dialog headless, so there is no assembly run): title, options
           (handle_token, modal, current_name, current_folder), filters, current_filter; the
           Response's uri (host, %XX) and current_filter -> path + index, the extension added
           when saving, cancel (1), no portal (error reply), no bus; canonical marshalling;
           FD_FOLDER (OpenFile with "directory", the folder as current_folder, no filters)
  print    tools/print_portal_bus.py's cases for the assembly's PaintCraft (script: Drucken...)
           and plattest --test printjob --case 0: PreparePrint / Print byte for byte, the PDF
           in the descriptor byte for byte (and checked like print_check.py), the results
Exit code 0 = passed.
"""
import argparse, os, socket, struct, subprocess, sys, tempfile, threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import print_check as pc  # noqa: E402

TMP = tempfile.mkdtemp(prefix="psbus")
MM = 72 / 25.4
TOKEN = 4242
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


# ------------------------------------------------------------------ D-Bus marshalling (generic)
ALIGN = {"y": 1, "g": 1, "v": 1, "n": 2, "q": 2, "b": 4, "i": 4, "u": 4, "h": 4, "s": 4, "o": 4, "a": 4,
         "x": 8, "t": 8, "d": 8, "(": 8, "{": 8}


def one_type(sig, i=0):
    """-> end index of the complete type at sig[i]"""
    c = sig[i]
    if c == "a":
        return one_type(sig, i + 1)
    if c in "({":
        depth = 0
        while True:
            if sig[i] in "({":
                depth += 1
            elif sig[i] in ")}":
                depth -= 1
                if depth == 0:
                    return i + 1
            i += 1
    return i + 1


def split(sig):
    out, i = [], 0
    while i < len(sig):
        e = one_type(sig, i)
        out.append(sig[i:e])
        i = e
    return out


class W:
    def __init__(self):
        self.b = bytearray()

    def pad(self, n):
        self.b += b"\0" * (-len(self.b) % n)

    def put(self, t, v):
        c = t[0]
        self.pad(ALIGN[c])
        if c == "y":
            self.b.append(v)
        elif c in "nq":
            self.b += struct.pack("<H", v & 0xFFFF)
        elif c in "biuh":
            self.b += struct.pack("<I", v & 0xFFFFFFFF)
        elif c in "xt":
            self.b += struct.pack("<Q", v & 0xFFFFFFFFFFFFFFFF)
        elif c == "d":
            self.b += struct.pack("<d", v)
        elif c in "so":
            e = v.encode() if isinstance(v, str) else v
            self.b += struct.pack("<I", len(e)) + e + b"\0"
        elif c == "g":
            e = v.encode()
            self.b += bytes([len(e)]) + e + b"\0"
        elif c == "v":
            vs, vv = v
            self.put("g", vs)
            self.put(vs, vv)
        elif c == "a":
            at = len(self.b)
            self.b += b"\0\0\0\0"
            et = t[1:]
            self.pad(ALIGN[et[0]])
            start = len(self.b)
            items = v.items() if et[0] == "{" else v
            for it in items:
                self.put(et, it)
            struct.pack_into("<I", self.b, at, len(self.b) - start)
        elif c in "({":
            for st, sv in zip(split(t[1:-1]), v):
                self.put(st, sv)
        else:
            raise ValueError(t)

    def all(self, sig, vals):
        for t, v in zip(split(sig), vals):
            self.put(t, v)
        return bytes(self.b)


class R:
    def __init__(self, b):
        self.b, self.p = b, 0

    def pad(self, n):
        self.p += -self.p % n

    def get(self, t):
        c = t[0]
        self.pad(ALIGN[c])
        b = self.b
        if c == "y":
            self.p += 1
            return b[self.p - 1]
        if c in "nq":
            self.p += 2
            return struct.unpack_from("<H", b, self.p - 2)[0]
        if c in "biuh":
            self.p += 4
            return struct.unpack_from("<I", b, self.p - 4)[0]
        if c in "xt":
            self.p += 8
            return struct.unpack_from("<Q", b, self.p - 8)[0]
        if c == "d":
            self.p += 8
            return struct.unpack_from("<d", b, self.p - 8)[0]
        if c in "so":
            n = struct.unpack_from("<I", b, self.p)[0]
            v = b[self.p + 4:self.p + 4 + n].decode()
            self.p += 5 + n
            return v
        if c == "g":
            n = b[self.p]
            v = b[self.p + 1:self.p + 1 + n].decode()
            self.p += 2 + n
            return v
        if c == "v":
            vs = self.get("g")
            return (vs, self.get(vs))
        if c == "a":
            n = struct.unpack_from("<I", b, self.p)[0]
            self.p += 4
            et = t[1:]
            self.pad(ALIGN[et[0]])
            end = self.p + n
            out = {} if et[0] == "{" else []
            while self.p < end:
                v = self.get(et)
                if et[0] == "{":
                    out[v[0]] = v[1]
                else:
                    out.append(v)
            return out
        if c in "({":
            return tuple(self.get(st) for st in split(t[1:-1]))
        raise ValueError(t)

    def all(self, sig):
        return [self.get(t) for t in split(sig)]


def fields_bytes(fields):
    fb = b""
    for code, sig, val in fields:
        fb += b"\0" * (-len(fb) % 8)
        w = W()
        w.b += bytes([code, 1, ord(sig), 0])
        w.put(sig, val)
        fb += bytes(w.b)
    return fb


def message(mtype, serial, fields, body=b""):
    fb = fields_bytes(fields)
    hdr = b"l" + bytes([mtype, 0, 1]) + struct.pack("<III", len(body), serial, len(fb)) + fb
    return hdr + b"\0" * (-len(hdr) % 8) + body


def parse(buf):
    """-> (msg dict, rest) or (None, buf); msg["raw"] = its bytes"""
    if len(buf) < 16:
        return None, buf
    blen, serial, flen = struct.unpack_from("<III", buf, 4)
    hend = 16 + flen
    total = hend + (-hend % 8) + blen
    if len(buf) < total:
        return None, buf
    m = {"type": buf[1], "serial": serial, "fields": {}, "raw": bytes(buf[:total])}
    r = R(buf)
    r.p = 16
    while r.p < hend:
        r.pad(8)
        code = buf[r.p]
        r.p += 1
        sig = r.get("g")
        m["fields"][code] = r.get(sig)
    m["body"] = bytes(buf[hend + (-hend % 8):total])
    return m, buf[total:]


# ------------------------------------------------------------------ the fake portal
HANDLE = "/org/freedesktop/portal/desktop/request/1_42/x"


def listen_unix(path, backlog=1):
    """a Unix socket listening at `path`; an already listening socket is returned as it is.  The
    fake buses bind before their thread starts: a program started right after t.start() could
    otherwise connect before the bind, find no bus and pass a "no service" check by accident"""
    if isinstance(path, socket.socket):
        return path
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(backlog)
    return srv


def serve(path, mode, log, raw):
    """one client connection; log: SASL lines, members and facts; raw: every message (bytes)"""
    srv = listen_unix(path, 1)
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
        raw.append(line)
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
        raw.append(m["raw"])
        member = m["fields"].get(3, "")
        log.append(member)
        sser += 1

        def reply(sig, body):
            return message(2, sser, [(5, "u", m["serial"]), (6, "s", ":1.42")] + ([(8, "g", sig)] if sig else []), body)

        def error(name):
            return message(3, sser, [(4, "s", name), (5, "u", m["serial"]), (6, "s", ":1.42")])

        def response(handle, code, results):
            body = W().all("ua{sv}", [code, results])
            return message(4, sser + 50, [(1, "o", handle), (2, "s", "org.freedesktop.portal.Request"),
                                          (3, "s", "Response"), (6, "s", ":1.42"), (8, "g", "ua{sv}")], body)
        sig = m["fields"].get(8, "")
        if member == "Hello":
            c.sendall(reply("s", W().all("s", [":1.42"])))
        elif member == "AddMatch":
            log.append("match " + R(m["body"]).all("s")[0])
            c.sendall(reply("", b""))
        elif member in ("OpenDirectory", "OpenURI"):
            args = R(m["body"]).all(sig)
            log.append(("args", member, args, m["body"]))
            if member == "OpenDirectory":
                link = os.readlink(f"/proc/self/fd/{fds[0]}") if fds else "?"
                log.append(f"sig {sig} unix_fds {m['fields'].get(9, 0)} fds {len(fds)} -> {link}")
                for d in fds:
                    os.close(d)
                fds = []
                if mode == "reject":
                    c.sendall(error("org.freedesktop.DBus.Error.UnknownMethod"))
                    continue
            else:
                log.append(f"sig {sig} uri {args[1]}")
            c.sendall(reply("o", W().all("o", [HANDLE])))
            if mode == "silent":
                continue
            c.sendall(response(HANDLE, 2 if mode == "fail" else 0, {}))
        elif member in ("OpenFile", "SaveFile"):
            args = R(m["body"]).all(sig)
            log.append(("args", member, args, m["body"]))
            if mode == "noportal":
                c.sendall(error("org.freedesktop.DBus.Error.UnknownMethod"))
                continue
            handle = f"/org/freedesktop/portal/desktop/request/1_42/{args[2].get('handle_token', ('s', '?'))[1]}"
            c.sendall(reply("o", W().all("o", [handle])))
            if mode == "cancel":
                c.sendall(response(handle, 1, {}))
                continue
            res = {"uris": ("as", [FD_URI[0]])}
            if FD_FILTER[0] is not None:
                res["current_filter"] = ("(sa(us))", FD_FILTER[0])
            c.sendall(response(handle, 0, res))
        elif member in ("PreparePrint", "Print"):
            log.append(f"{member} sig {sig} iface {m['fields'].get(2)} path {m['fields'].get(1)}")
            if mode == "noportal":
                c.sendall(error("org.freedesktop.DBus.Error.UnknownMethod"))
                continue
            args = R(m["body"]).all(sig)
            log.append(("args", member, args, m["body"]))
            nreq += 1
            handle = f"/org/freedesktop/portal/desktop/request/1_42/r{nreq}"
            c.sendall(reply("o", W().all("o", [handle])))
            if member == "PreparePrint":
                if mode == "cancel":
                    c.sendall(response(handle, 1, {}))
                    continue
                setup = {"PPDName": ("s", "Letter"), "Name": ("s", "na_letter"), "DisplayName": ("s", "US Letter"),
                         "Width": ("d", 215.9), "Height": ("d", 279.4), "MarginTop": ("d", 6.35),
                         "MarginBottom": ("d", 6.35), "MarginLeft": ("d", 6.35), "MarginRight": ("d", 6.35),
                         "Orientation": ("s", "landscape")}
                c.sendall(response(handle, 0, {"settings": ("a{sv}", {"n-copies": ("s", "2")}),
                                               "page-setup": ("a{sv}", setup), "token": ("u", TOKEN)}))
            else:
                data = b""
                if fds:
                    os.lseek(fds[0], 0, os.SEEK_SET)
                    while True:
                        chunk = os.read(fds[0], 1 << 20)
                        if not chunk:
                            break
                        data += chunk
                log.append(("fd", m["fields"].get(9, 0), len(fds), data))
                for d in fds:
                    os.close(d)
                fds = []
                c.sendall(response(handle, 2 if mode == "printfail" else 0, {}))


FD_URI = ["file:///tmp/x"]
FD_FILTER = [None]
RUNS = [0]


def run(cmd, mode, bus=True, env_extra=None, env_drop=(), wait=5):
    RUNS[0] += 1
    sock = os.path.join(TMP, f"bus-{RUNS[0]}-{mode}")
    log, raw = [], []
    env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}" if bus else "unix:path=/nonexistent")
    for k in ("jfgui_OPEN_DRYRUN", "jfgui_PRINT_TO", "jfgui_PRINT_PORTAL", "jfgui_PRINT_SMASK") + tuple(env_drop):
        env.pop(k, None)
    env.update(env_extra or {})
    t = None
    if bus:
        t = threading.Thread(target=serve, args=(listen_unix(sock, 1), mode, log, raw), daemon=True)
        t.start()
        while not os.path.exists(sock):
            pass
    p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=120, cwd=ROOT)
    if t:
        t.join(wait)
    return p.returncode, log, raw, p.stdout + p.stderr


def strs(log):
    return [l for l in log if isinstance(l, str)]


def args_of(log, member):
    return [a for a in log if isinstance(a, tuple) and a[0] == "args" and a[1] == member]


# ------------------------------------------------------------------ open folder: asm vs Rust
def suite_open(ref, rs):
    target = os.path.join(TMP, "Ordner mit Ä")
    os.mkdir(target)
    file = os.path.join(target, "bild.png")
    open(file, "wb").close()
    asm = lambda p: [f"./{ref}/linux/gfxtest", "--shot", os.path.join(TMP, "x.png"), "--open", p]
    rust = lambda p: [f"./{rs}/linux/plattest", "--test", "portal", "--mode", "open", "--path", p]
    uri = "file://" + target.replace(" ", "%20").replace("Ä", "%C3%84")
    cases = [("ok", target, True), ("ok", file, True), ("reject", target, True), ("nofd", target, True),
             ("fail", target, True), ("silent", target, True), ("ok", os.path.join(TMP, "gibt es nicht"), True),
             ("ok", target, False)]
    for mode, path, bus in cases:
        name = f"open {mode} {os.path.basename(path)}{'' if bus else ' (no bus)'}"
        ra, la, wa, oa = run(asm(path), mode, bus)
        rr, lr, wr, orr = run(rust(path), mode, bus)
        check(f"{name}: exit code asm {ra} = Rust {rr}", ra == rr, (oa[-300:], orr[-300:]))
        check(f"{name}: SASL + {len(wa)} messages byte for byte", wa == wr and strs(la) == strs(lr),
              (strs(la), strs(lr)))
    # the facts openfolder_bus.py checks, on the Rust side
    rr, lr, _, _ = run(rust(target), "ok")
    check("open: OpenDirectory with an O_PATH descriptor of the folder",
          rr == 0 and f"sig sha{{sv}} unix_fds 1 fds 1 -> {target}" in lr and "NEGOTIATE_UNIX_FD" in lr
          and any(isinstance(l, str) and l.startswith("match ") and "request/1_42/jfgui" in l for l in lr), lr)
    rr, lr, _, _ = run(rust(target), "reject")
    check("open: rejected OpenDirectory -> OpenURI(file://, %XX)", rr == 0 and f"sig ssa{{sv}} uri {uri}" in lr, lr)
    # dry run: printed, nothing sent
    rr, lr, _, out = run(rust(target), "ok", env_extra={"jfgui_OPEN_DRYRUN": "1"}, wait=0.5)
    ra, la, _, oa = run(asm(target), "ok", env_extra={"jfgui_OPEN_DRYRUN": "1"}, wait=0.5)
    line = [l for l in out.splitlines() if l.startswith("plat_open_folder:")]
    aline = [l for l in oa.splitlines() if l.startswith("plat_open_folder:")]
    check("open: jfgui_OPEN_DRYRUN=1 prints the call (as the assembly), no bus contact",
          rr == 0 and ra == 0 and line and line == aline and not any(l.startswith("AUTH") for l in strs(lr)),
          (out, oa, lr))


# ------------------------------------------------------------------ open url: Rust
def suite_url(rs):
    rust = lambda u: [f"./{rs}/linux/plattest", "--test", "portal", "--mode", "url", "--url", u]
    u = "https://example.org/a b?x=1&y=ä"
    rr, lr, wr, _ = run(rust(u), "ok")
    a = args_of(lr, "OpenURI")
    ok = rr == 0 and a and a[0][2][0] == "" and a[0][2][1] == u and a[0][2][2] == {"handle_token": ("s", "jfgui1")}
    check("url: OpenURI(\"\", url as given, {handle_token})", ok, lr)
    check("url: canonical marshalling", a and W().all("ssa{sv}", a[0][2]) == a[0][3], a)
    check("url: no descriptor negotiation", "NEGOTIATE_UNIX_FD" not in lr, lr)
    rr, _, _, _ = run(rust(u), "fail")
    check("url: Response 2 -> -1", rr == 1)
    rr, lr, _, _ = run(rust(""), "ok", wait=0.5)
    check("url: empty -> -1 without a bus connection", rr == 1 and strs(lr) in ([], ["no connection"]), lr)
    rr, _, _, _ = run(rust(u), "ok", bus=False)
    check("url: no bus -> -1", rr == 1)


# ------------------------------------------------------------------ file dialog: Rust
EXTS = "png=PNG-Bild|jpg;jpeg=JPEG-Bild|bmp"


def suite_filedlg(rs):
    folder = os.path.join(TMP, "Bilder ä")
    os.makedirs(folder, exist_ok=True)

    def rust(flags, path, exts=EXTS, expect=1):
        return [f"./{rs}/linux/plattest", "--test", "portal", "--mode", "filedlg", "--flags", str(flags), "--path", path,
                "--exts", exts, "--expect", str(expect)]

    def result(out):
        ln = [l for l in out.splitlines() if l.startswith("filedlg: ")]
        if not ln:
            return None
        r, idx, *p = ln[0][9:].split(" ", 2)
        return int(r), int(idx), (p[0] if p else "")

    def g(ext):  # GTK globs are case-sensitive: plat sends every letter as [xX] (IMG_0001.JPG)
        return (0, "*." + "".join(f"[{c}{c.upper()}]" if c.isalpha() else c for c in ext))
    flt_all = ("Alle unterstützten Dateien", [g("png"), g("jpg"), g("jpeg"), g("bmp")])
    flt = [("PNG-Bild", [g("png")]), ("JPEG-Bild", [g("jpg"), g("jpeg")]), ("BMP-Datei", [g("bmp")])]
    # open
    FD_URI[0] = "file://" + (folder + "/x y.jpeg").replace(" ", "%20").replace("ä", "%C3%A4")
    FD_FILTER[0] = flt[1]
    rr, lr, wr, out = run(rust(0, folder + "/alt.png"), "ok")
    a = args_of(lr, "OpenFile")
    want_opts = {"handle_token": ("s", "jfgui1"), "modal": ("b", 1), "current_folder": ("ay", list(folder.encode() + b"\0")),
                 "filters": ("a(sa(us))", [flt_all] + flt), "current_filter": ("(sa(us))", flt_all)}
    got = a[0][2] if a else None
    check("filedlg open: OpenFile(\"\", \"Öffnen\", options)", got is not None and got[0] == "" and got[1] == "Öffnen", got)
    if got:
        opts = dict(got[2])
        opts["filters"] = ("a(sa(us))", [(n, [tuple(p) for p in ps]) for n, ps in opts["filters"][1]])
        opts["current_filter"] = ("(sa(us))", (opts["current_filter"][1][0], [tuple(p) for p in opts["current_filter"][1][1]]))
        check("filedlg open: handle_token, modal, current_folder, all-files filter first, current_filter",
              opts == want_opts, (opts, want_opts))
        check("filedlg open: canonical marshalling", W().all("ssa{sv}", got) == a[0][3])
    check("filedlg open: path from the URI (%XX decoded), index of the chosen filter",
          rr == 0 and result(out) == (1, 1, folder + "/x y.jpeg"), (rr, out))
    # save: current_name, the selected filter, the extension added
    FD_URI[0] = "file://localhost" + folder + "/neu"
    FD_FILTER[0] = flt[0]
    rr, lr, wr, out = run(rust(1 | 2 << 8, folder + "/bild.bmp"), "ok")
    a = args_of(lr, "SaveFile")
    got = a[0][2] if a else None
    ok = got is not None and got[1] == "Speichern unter" and got[2].get("current_name") == ("s", "bild.bmp") \
        and got[2]["current_filter"][1][0] == "BMP-Datei" and len(got[2]["filters"][1]) == 3
    check("filedlg save: SaveFile(\"\", \"Speichern unter\", current_name, filters without 'all', current_filter = index 2)",
          ok, got)
    check("filedlg save: file://localhost/... -> path, PNG chosen -> \".png\" added, index 0",
          rr == 0 and result(out) == (1, 0, folder + "/neu.png"), (rr, out))
    # save with an extension already there / an unknown current filter
    FD_URI[0] = "file://" + folder + "/a.b/c.tif"
    FD_FILTER[0] = ("Unbekannt", [(0, "*.x")])
    rr, lr, _, out = run(rust(1 | 7 << 8, "nurname.png"), "ok")
    got = args_of(lr, "SaveFile")
    check("filedlg save: an index past the list selects entry 0, a relative path gives only current_name",
          got and got[0][2][2]["current_filter"][1][0] == "PNG-Bild" and "current_folder" not in got[0][2][2]
          and got[0][2][2].get("current_name") == ("s", "nurname.png"), got)
    check("filedlg save: a name with an extension stays, unknown filter -> index 0",
          rr == 0 and result(out) == (1, 0, folder + "/a.b/c.tif"), out)
    # no extension list
    FD_URI[0] = "file:///tmp/z"
    FD_FILTER[0] = None
    rr, lr, _, out = run(rust(0, "", exts=""), "ok")
    got = args_of(lr, "OpenFile")
    check("filedlg: no extensions -> no filters", got and set(got[0][2][2]) == {"handle_token", "modal"} and result(out) == (1, 0, "/tmp/z"),
          (got, out))
    # FD_FOLDER: OpenFile with "directory", starting in the folder itself, no filters, nothing added
    FD_URI[0] = "file://" + folder.replace(" ", "%20").replace("ä", "%C3%A4")
    rr, lr, _, out = run(rust(2, folder), "ok")
    got = args_of(lr, "OpenFile")
    opts = dict(got[0][2][2]) if got else {}
    check("filedlg folder: OpenFile(\"\", \"Ordner auswählen\", directory, current_folder = the folder, no filters)",
          bool(got) and got[0][2][1] == "Ordner auswählen" and opts.get("directory") == ("b", 1)
          and opts.get("current_folder") == ("ay", list(folder.encode() + b"\0")) and "filters" not in opts, got)
    check("filedlg folder: the folder's path, index 0, no extension added", rr == 0 and result(out) == (1, 0, folder), out)
    # cancel, no portal, no bus, a non-file URI
    rr, lr, _, out = run(rust(0, "/tmp/q.png", expect=0), "cancel")
    check("filedlg: cancelled -> 0, the path unchanged", rr == 0 and result(out) == (0, 0, "/tmp/q.png"), out)
    rr, lr, _, out = run(rust(0, "/tmp/q.png", expect=-1), "noportal")
    check("filedlg: no FileChooser (error reply) -> -1", rr == 0 and result(out) == (-1, 0, "/tmp/q.png"), out)
    rr, lr, _, out = run(rust(0, "/tmp/q.png", expect=-1), "ok", bus=False)
    check("filedlg: no bus -> -1", rr == 0 and result(out) == (-1, 0, "/tmp/q.png"), out)
    FD_URI[0] = "https://example.org/x"
    rr, lr, _, out = run(rust(0, "/tmp/q.png", expect=0), "ok")
    check("filedlg: a non-file URI -> 0", rr == 0 and result(out) == (0, 0, "/tmp/q.png"), out)


# ------------------------------------------------------------------ print: asm vs Rust
def suite_print(ref, rs):
    out = ref  # PaintCraft's --config goes below the reference folder (this worktree's copy)
    work = Path(TMP) / "print"
    work.mkdir()
    px = pc.make_pixels()
    png = work / "druck_test.png"
    pc.write_png(png, px)

    def asm(error=None, tag="x"):
        scr = work / f"asm_{tag}.uis"
        pc.script(scr, [], None, error)
        return [f"./{ref}/linux/PaintCraft", "--config", f"{out}/cfg", "--open", str(png), "--script", str(scr)]

    def rust(expect=0):
        return [f"./{rs}/linux/plattest", "--test", "printjob", "--case", "0", "--expect", str(expect)]

    portal = {"jfgui_PRINT_PORTAL": "1"}
    ra, la, wa, oa = run(asm(tag="ok"), "ok", env_extra=portal, wait=25)
    rr, lr, wr, orr = run(rust(), "ok", env_extra=portal, wait=25)
    check("print ok: Rust printed", rr == 0, orr[-400:])
    check(f"print ok: SASL + {len(wa)} messages byte for byte (Hello, AddMatch, PreparePrint, AddMatch, Print)",
          wa == wr and len(wa) >= 8, (strs(la), strs(lr)))
    fa = [a for a in la if isinstance(a, tuple) and a[0] == "fd"]
    fr = [a for a in lr if isinstance(a, tuple) and a[0] == "fd"]
    check("print ok: one descriptor, the PDF byte for byte", fa and fr and fr[0][1] == 1 and fr[0][2] == 1 and fa[0][3] == fr[0][3],
          (len(fa and fa[0][3]), len(fr and fr[0][3])))
    if fr and fr[0][3]:
        lay = pc.layout(0, 0, 0, 100, 1, 5, page=(279.4 * MM, 215.9 * MM), insets=(6.35 * MM,) * 4)
        pc.check_ours("print ok: portal PDF (Letter landscape from the page setup)", fr[0][3], px, lay)
    prep = args_of(lr, "PreparePrint")
    if prep:
        parent, title, settings, setup, opts = prep[0][2]
        check("print ok: PreparePrint suggests A4 portrait, modal, handle_token",
              title == "druck_test.png" and settings == {"orientation": ("s", "portrait")}
              and setup.get("Name") == ("s", "iso_a4") and setup.get("Width") == ("d", 210.0) and setup.get("Height") == ("d", 297.0)
              and opts.get("modal") == ("b", 1) and opts.get("handle_token", ("s", ""))[1].startswith("jfgui"), prep[0][2])
        check("print ok: PreparePrint canonical marshalling", W().all("ssa{sv}a{sv}a{sv}", prep[0][2]) == prep[0][3])
    pr = args_of(lr, "Print")
    check("print ok: Print(parent, title, h 0, {handle_token, modal, token})",
          pr and pr[0][2][1] == "druck_test.png" and pr[0][2][2] == 0 and pr[0][2][3].get("token") == ("u", TOKEN), pr)
    cases = [("cancel", -14, None, True, True), ("noportal", -9, pc.MSG_UNSUPPORTED, True, True),
             ("printfail", -13, pc.MSG_FAIL, True, True), ("nofd", -9, pc.MSG_UNSUPPORTED, True, True),
             ("ok", -9, pc.MSG_UNSUPPORTED, False, True), ("ok", -9, pc.MSG_UNSUPPORTED, True, False)]
    for mode, want, err, penv, bus in cases:
        name = f"print {mode}{'' if penv else ' (no jfgui_PRINT_PORTAL)'}{'' if bus else ' (no bus)'}"
        env = portal if penv else {}
        w = 3 if not penv else 25
        ra, la, wa, oa = run(asm(err, tag=mode), mode, bus=bus, env_extra=env, wait=w)
        rr, lr, wr, orr = run(rust(want), mode, bus=bus, env_extra=env, wait=w)
        check(f"{name}: Rust -> {want}", rr == 0, orr[-300:])
        check(f"{name}: SASL + {len(wa)} messages byte for byte", wa == wr, (strs(la), strs(lr)))
        if not penv:
            check(f"{name}: no bus contact", not any(l.startswith("AUTH") for l in strs(lr)), lr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", default=os.environ.get("jfgui_REF_REL", "build/rwref"))
    ap.add_argument("--rs", default="build")
    ap.add_argument("--only", default="open,url,filedlg,print")
    a = ap.parse_args()
    only = a.only.split(",")
    if "open" in only:
        suite_open(a.ref, a.rs)
    if "url" in only:
        suite_url(a.rs)
    if "filedlg" in only:
        suite_filedlg(a.rs)
    if "print" in only:
        suite_print(a.ref, a.rs)
    print("portal_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
