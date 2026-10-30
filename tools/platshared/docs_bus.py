#!/usr/bin/env python3
"""docs_bus.py -- lasting document IDs of Flatpak's documents portal (lib/plat/linux_docs.rs,
plat::persistent_name) against a fake portal on a private bus, or with --real against
xdg-document-portal itself.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/docs_bus.py [--rs build] [--real]

Fake: a scratch $XDG_RUNTIME_DIR whose doc/ folder plays the portal's mount: a1b2c3d4/brief.txt
is the document the file chooser handed out (a hard link of the host's file).  The fake bus
(SASL EXTERNAL, NEGOTIATE_UNIX_FD, SCM_RIGHTS) answers org.freedesktop.portal.Documents
GetMountPoint (ay) and Add(h, b, b): the descriptor must be O_PATH and name the document, both
flags true; it makes <ID>/brief.txt for a lasting ID (the same file: the same ID) and refuses
refused1/.  plattest --test docs-bus checks persistent_name; this script the calls it saw.

--real: the script runs itself again inside dbus-run-session: xdg-document-portal (activated,
with xdg-permission-store) mounts its FUSE file system in the scratch $XDG_RUNTIME_DIR, keeps its
store in a scratch $XDG_DATA_HOME; the script exports the host's file as a session document
(Add, persistent false: flatpak's file forwarding) and as a lasting one (persistent true: the
file chooser portal); plattest checks persistent_name against the real answers (a document of
the mount keeps its own ID: the portal answers a descriptor on its mount with that ID and
refuses reuse_existing false, so an app cannot make a session document lasting).  Then plattest
runs in a bwrap sandbox (an empty root, /usr, /etc, /tmp, the build, a /.flatpak-info) with a
manifest of fs.user only and chooses a file through xdg-desktop-portal's FileChooser (activated
with XDG_DESKTOP_PORTAL_DIR / XDG_CURRENT_DESKTOP=jftest: only the fake chooser of this script
behind it, answering with a host file): a document of the mount, read without a grant.  The
document portal is stopped and started again (its mount gone and back): plattest --phase 2
reads and writes the host's file through the lasting document (the session's is gone), and in
the sandbox the chosen file again without a dialog.  The portals and the mount are removed at
the end.
Exit code 0 = passed.
"""
import argparse, fcntl, os, shutil, signal, socket, struct, subprocess, sys, tempfile, threading, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, listen_unix, message, parse  # noqa: E402

NAME = "org.freedesktop.portal.Documents"
PATH = "/org/freedesktop/portal/documents"
O_PATH = 0o10000000
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def plattest(exe, env, *args):
    r = subprocess.run([str(exe), "--test", "docs-bus", *args], env=env, capture_output=True, text=True, timeout=120)
    print(r.stdout.strip())
    if r.stderr.strip():
        print(r.stderr.strip())
    return r


def clean_env(**extra):
    env = {k: v for k, v in os.environ.items() if not k.startswith("jfgui_")}
    env.update(extra)
    return env


# ------------------------------------------------------------------ the fake portal
def serve_one(c, mount, rec, lock):
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
        if line.startswith(b"AUTH"):
            c.sendall(b"OK 0123456789abcdef0123456789abcdef\r\n")
        elif line.startswith(b"NEGOTIATE_UNIX_FD"):
            with lock:
                rec["negotiated"] += 1
            c.sendall(b"AGREE_UNIX_FD\r\n")
        elif line.startswith(b"BEGIN"):
            break
    fds, ser = [], 100
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
        fl = m["fields"]
        member, sig = fl.get(3, ""), fl.get(8, "")
        ser += 1

        def reply(s="", body=b""):
            return message(2, ser, [(5, "u", m["serial"]), (6, "s", ":1.42")] + ([(8, "g", s)] if s else []), body)

        def error(name):
            return message(3, ser, [(4, "s", name), (5, "u", m["serial"]), (6, "s", ":1.42")])
        if m["type"] != 1:
            continue
        if member == "Hello":
            c.sendall(reply("s", W().all("s", [":1.42"])))
        elif member == "GetMountPoint":
            with lock:
                rec["mount"].append((fl.get(6), fl.get(1), fl.get(2)))
            c.sendall(reply("ay", W().all("ay", [list(mount.encode() + b"\0")])))
        elif member == "Add":
            args = R(m["body"]).all(sig)
            fd = fds.pop(0) if fds else -1
            info = {"sig": sig, "unix_fds": fl.get(9, 0), "args": args, "dest": fl.get(6), "path": fl.get(1), "iface": fl.get(2)}
            if fd >= 0:
                info["o_path"] = bool(fcntl.fcntl(fd, fcntl.F_GETFL) & O_PATH)
                info["target"] = os.readlink(f"/proc/self/fd/{fd}")
                st = os.fstat(fd)
                os.close(fd)
            with lock:
                rec["add"].append(info)
            target = info.get("target", "")
            if fd < 0 or "/refused1/" in target:
                c.sendall(error("org.freedesktop.portal.Error.NotAllowed"))
                continue
            with lock:
                doc = rec["ids"].setdefault(st.st_ino, f"p{len(rec['ids']) + 1:07d}")
            name = os.path.basename(target)
            os.makedirs(os.path.join(mount, doc), exist_ok=True)
            lasting = os.path.join(mount, doc, name)
            if not os.path.exists(lasting):
                os.link(target, lasting)
            c.sendall(reply("s", W().all("s", [doc])))
        else:
            c.sendall(error("org.freedesktop.DBus.Error.UnknownMethod"))


def serve(path, mount, rec, stop):
    srv = listen_unix(path, 8)
    srv.settimeout(0.2)
    lock = threading.Lock()
    while not stop.is_set():
        try:
            c, _ = srv.accept()
        except socket.timeout:
            continue
        rec["connections"] += 1
        threading.Thread(target=serve_one, args=(c, mount, rec, lock), daemon=True).start()
    srv.close()


def fake(exe):
    with tempfile.TemporaryDirectory() as d:
        run, host = os.path.join(d, "run"), os.path.join(d, "host")
        mount = os.path.join(run, "doc")
        for p in (host, os.path.join(mount, "a1b2c3d4"), os.path.join(mount, "refused1")):
            os.makedirs(p, mode=0o700)
        Path(host, "brief.txt").write_bytes(b"Brief vom Host\n")
        doc = os.path.join(mount, "a1b2c3d4", "brief.txt")
        os.link(os.path.join(host, "brief.txt"), doc)
        os.link(os.path.join(host, "brief.txt"), os.path.join(mount, "refused1", "brief.txt"))
        rec = {"connections": 0, "negotiated": 0, "mount": [], "add": [], "ids": {}}
        stop = threading.Event()
        sock = os.path.join(d, "bus")
        t = threading.Thread(target=serve, args=(listen_unix(sock, 8), mount, rec, stop), daemon=True)
        t.start()
        r = plattest(exe, clean_env(DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}", XDG_RUNTIME_DIR=run), "--doc", doc)
        stop.set()
        t.join(5)
    check("plattest --test docs-bus passed (fake portal)", r.returncode == 0, r.stderr.strip())
    adds = rec["add"]
    check("GetMountPoint once per process, on the portal's object",
          rec["mount"] == [(NAME, PATH, NAME)], rec["mount"])
    check("descriptor passing negotiated on every connection", rec["negotiated"] == rec["connections"] >= 4, rec)
    check("Add(h, b, b) for the document (twice), the lasting name and the refused one",
          len(adds) == 4 and all(a["sig"] == "hbb" and a["unix_fds"] == 1 and a["args"][0] == 0 for a in adds), adds)
    check("reuse_existing and persistent true", all(a["args"][1] == 1 and a["args"][2] == 1 for a in adds), adds)
    check("an O_PATH descriptor of the file", all(a.get("o_path") for a in adds), adds)
    check("on org.freedesktop.portal.Documents at /org/freedesktop/portal/documents",
          all((a["dest"], a["path"], a["iface"]) == (NAME, PATH, NAME) for a in adds), adds)
    check("no bus for names that are no file documents (5 connections: doc twice, kept, missing, refused)",
          rec["connections"] == 5, rec["connections"])


# ------------------------------------------------------------------ the real portal
class Bus:
    """a minimal client of the session bus (descriptor passing)"""

    def __init__(self):
        a = os.environ["DBUS_SESSION_BUS_ADDRESS"]
        part = a.split(";")[0][5:]
        kv = dict(x.split("=", 1) for x in part.split(","))
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.connect(kv["path"] if "path" in kv else "\0" + kv["abstract"])
        self.s.settimeout(30)
        self.s.sendall(b"\0AUTH EXTERNAL " + str(os.getuid()).encode().hex().encode() + b"\r\n")
        assert self._line().startswith(b"OK")
        self.s.sendall(b"NEGOTIATE_UNIX_FD\r\n")
        assert self._line().startswith(b"AGREE_UNIX_FD")
        self.s.sendall(b"BEGIN\r\n")
        self.buf, self.serial = b"", 0
        self.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello")

    def _line(self):
        b = b""
        while not b.endswith(b"\r\n"):
            b += self.s.recv(1)
        return b

    def call(self, dest, path, iface, member, sig="", args=(), fds=()):
        self.serial += 1
        fields = [(1, "o", path), (2, "s", iface), (3, "s", member), (6, "s", dest)]
        if sig:
            fields.append((8, "g", sig))
        if fds:
            fields.append((9, "u", len(fds)))
        m = message(1, self.serial, fields, W().all(sig, list(args)) if sig else b"")
        if fds:
            socket.send_fds(self.s, [m], list(fds))
        else:
            self.s.sendall(m)
        while True:
            r, self.buf = parse(self.buf)
            if r is None:
                d = self.s.recv(65536)
                if not d:
                    raise OSError("bus closed")
                self.buf += d
                continue
            if r["type"] in (2, 3) and r["fields"].get(5) == self.serial:
                if r["type"] == 3:
                    raise OSError(f"{member}: {r['fields'].get(4)}")
                s = r["fields"].get(8, "")
                return R(r["body"]).all(s) if s else []


def portal_pid(bus):
    return bus.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                    "GetConnectionUnixProcessID", "s", [NAME])[0]


def stop_portal(bus, mount):
    try:
        pid = portal_pid(bus)
    except OSError:
        pid = None
    if pid:
        os.kill(pid, signal.SIGTERM)
        for _ in range(50):
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.1)
    subprocess.run(["fusermount3", "-u", "-z", mount], capture_output=True)
    return pid


CHOOSER = "org.freedesktop.impl.portal.desktop.jftest"


def chooser_backend(host_file, ready, rec):
    """the file chooser behind xdg-desktop-portal: OpenFile answers with the host's file"""
    b = Bus()
    b.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", "su", [CHOOSER, 4])
    ready.set()
    ser = 500
    while True:
        m, b.buf = parse(b.buf)
        if m is None:
            try:
                data = b.s.recv(65536)
            except OSError:
                return
            if not data:
                return
            b.buf += data
            continue
        fl = m["fields"]
        if m["type"] != 1:
            continue
        ser += 1
        rec.append((fl.get(2), fl.get(3)))
        if fl.get(3) == "OpenFile":
            res = {"uris": ("as", ["file://" + host_file]), "writable": ("b", True)}
            b.s.sendall(message(2, ser, [(5, "u", m["serial"]), (6, "s", fl.get(7, "")), (8, "g", "ua{sv}")], W().all("ua{sv}", [0, res])))
        else:
            b.s.sendall(message(3, ser, [(4, "s", "org.freedesktop.DBus.Error.UnknownMethod"), (5, "u", m["serial"]), (6, "s", fl.get(7, ""))]))


def sandboxed(exe, d, *args):
    """plattest in a sandbox of bwrap: an empty root with /usr, /etc, /tmp, the build and a
    /.flatpak-info (what xdg-desktop-portal and plat::linux_desk::in_flatpak look for)"""
    cmd = ["bwrap", "--ro-bind", "/usr", "/usr", "--symlink", "usr/lib", "/lib", "--symlink", "usr/lib64", "/lib64",
           "--symlink", "usr/bin", "/bin", "--ro-bind", "/etc", "/etc", "--proc", "/proc", "--dev", "/dev",
           "--bind", "/tmp", "/tmp", "--ro-bind", str(exe.parent), str(exe.parent),
           "--ro-bind", os.path.join(d, "flatpak-info"), "/.flatpak-info", "--", str(exe), "--test", "docs-bus", "--mode", "chooser", *args]
    r = subprocess.run(cmd, env=clean_env(), capture_output=True, text=True, timeout=120)
    print(r.stdout.strip())
    if r.stderr.strip():
        print(r.stderr.strip()[-1500:])
    return r


def real_inner(exe, d):
    run, host = os.path.join(d, "run"), os.path.join(d, "host")
    # the bus (and the portals it starts) has the scratch folders: never the user's own
    if os.environ.get("XDG_RUNTIME_DIR") != run or os.environ.get("XDG_DATA_HOME") != os.path.join(d, "data"):
        check("the scratch XDG_RUNTIME_DIR / XDG_DATA_HOME", False, os.environ.get("XDG_RUNTIME_DIR"))
        return
    Path(host, "brief.txt").write_bytes(b"Brief vom Host\n")
    Path(host, "gewaehlt.txt").write_bytes(b"Brief vom Host\n")
    seen, ready = [], threading.Event()
    threading.Thread(target=chooser_backend, args=(os.path.join(host, "gewaehlt.txt"), ready, seen), daemon=True).start()
    ready.wait(10)
    bus = Bus()
    mount = bytes(bus.call(NAME, PATH, NAME, "GetMountPoint")[0]).rstrip(b"\0").decode()
    if not check("the real portal mounts in the scratch XDG_RUNTIME_DIR", mount == os.path.join(run, "doc"), mount):
        stop_portal(bus, mount)
        return
    try:
        # a document for this session (persistent false: flatpak's file forwarding, "open with")
        # and a lasting one (persistent true)
        def add(persistent):
            fd = os.open(os.path.join(host, "brief.txt"), os.O_PATH | os.O_CLOEXEC)
            try:
                return bus.call(NAME, PATH, NAME, "Add", "hbb", [0, 1, persistent], [fd])[0]
            finally:
                os.close(fd)
        session, lasting = add(0), add(1)
        doc, kept = os.path.join(mount, session, "brief.txt"), os.path.join(mount, lasting, "brief.txt")
        check("a session and a lasting document of the host's file", session != lasting
              and Path(doc).read_bytes() == Path(kept).read_bytes() == b"Brief vom Host\n", (doc, kept))
        env = clean_env()
        r = plattest(exe, env, "--mode", "real", "--doc", doc, "--lasting", kept)
        check("plattest --test docs-bus passed (real portal)", r.returncode == 0, r.stderr.strip())
        # the file chooser portal (xdg-desktop-portal, the fake chooser behind it) for an app in a
        # sandbox: plattest's portal dialog, read with fs.user and no grant
        r = sandboxed(exe, d, "--phase", "1")
        chosen = next((ln[7:] for ln in r.stdout.splitlines() if ln.startswith("CHOSEN ")), "")
        check("plattest in a sandbox: the portal's file dialog, fs.user (xdg-desktop-portal)", r.returncode == 0, r.stderr.strip()[-300:])
        check("the chooser was asked (OpenFile)", ("org.freedesktop.impl.portal.FileChooser", "OpenFile") in seen, seen)
        # the portal stopped and started again (as after a log-out or a reboot)
        pid = stop_portal(bus, mount)
        check("the portal stopped, its mount gone", pid and not os.path.exists(kept), (pid, os.path.exists(kept)))
        again = bytes(bus.call(NAME, PATH, NAME, "GetMountPoint")[0]).rstrip(b"\0").decode()
        check("the portal started again", again == mount and portal_pid(bus) != pid, again)
        r = plattest(exe, env, "--mode", "real", "--phase", "2", "--doc", doc, "--lasting", kept)
        check("plattest --phase 2: the lasting document after the restart, the session's gone", r.returncode == 0, r.stderr.strip())
        check("the host's file written through the lasting document",
              Path(host, "brief.txt").read_bytes() == b"geschrieben nach dem Neustart\n", Path(host, "brief.txt").read_bytes())
        r = sandboxed(exe, d, "--phase", "2", "--chosen", chosen)
        check("plattest in the sandbox after the restart: the chosen file without a dialog", r.returncode == 0 and bool(chosen), r.stderr.strip()[-300:])
        check("the chosen file written from the sandbox",
              Path(host, "gewaehlt.txt").read_bytes() == b"im Sandkasten gespeichert\n", Path(host, "gewaehlt.txt").read_bytes())
    finally:
        stop_portal(bus, mount)
        for n in ("org.freedesktop.portal.Desktop", "org.freedesktop.impl.portal.PermissionStore"):
            try:
                pid = bus.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                               "GetConnectionUnixProcessID", "s", [n])[0]
                os.kill(pid, signal.SIGTERM)
            except OSError:
                pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    ap.add_argument("--real", action="store_true")
    ap.add_argument("--inner", default="")
    a = ap.parse_args()
    exe = (Path(a.rs) / "linux" / "plattest").resolve()
    if a.inner:
        real_inner(exe, a.inner)
    elif a.real:
        if not shutil.which("dbus-run-session") or not os.path.exists("/usr/libexec/xdg-document-portal"):
            print("SKIP --real: dbus-run-session or xdg-document-portal missing")
            return 0
        if not shutil.which("bwrap") or not os.path.exists("/usr/libexec/xdg-desktop-portal"):
            print("SKIP --real: bwrap or xdg-desktop-portal missing")
            return 0
        d = tempfile.mkdtemp(prefix="docsbus")
        for p in ("run", "data", "host", "portals"):
            os.makedirs(os.path.join(d, p), mode=0o700)
        # xdg-desktop-portal with only our chooser (UseIn of XDG_CURRENT_DESKTOP); the sandbox's app
        Path(d, "portals", "jftest.portal").write_text(f"[portal]\nDBusName={CHOOSER}\n"
                                                       "Interfaces=org.freedesktop.impl.portal.FileChooser\nUseIn=jftest\n")
        Path(d, "flatpak-info").write_text("[Application]\nname=org.jflib.plattest\nruntime=runtime/org.freedesktop.Platform/x86_64/26.08\n")
        env = dict(os.environ, XDG_RUNTIME_DIR=os.path.join(d, "run"), XDG_DATA_HOME=os.path.join(d, "data"),
                   XDG_CURRENT_DESKTOP="jftest", XDG_DESKTOP_PORTAL_DIR=os.path.join(d, "portals"))
        for k in ("DISPLAY", "WAYLAND_DISPLAY"):
            env.pop(k, None)
        try:
            r = subprocess.run(["dbus-run-session", "--", sys.executable, os.path.abspath(__file__), "--rs", a.rs, "--inner", d],
                               env=env, timeout=300)
            check("the real run inside dbus-run-session", r.returncode == 0, r.returncode)
        finally:
            subprocess.run(["fusermount3", "-u", "-z", os.path.join(d, "run", "doc")], capture_output=True)
            shutil.rmtree(d, ignore_errors=True)
    else:
        fake(exe)
    print("docs_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
