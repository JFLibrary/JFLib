"""bc_cross.py -- BackupCraft across Windows and Linux (WSL): a Windows server backs up a Linux
agent and restores onto it; a Linux server does the same with a Windows agent.

    python tools/bc_cross.py [--distro Ubuntu-24.04] [--keep]

Needs build/win and build/linux (python build_rs.py app BackupCraft --target both) and python3
in the distribution.  Everything runs on loopback ports with temporary data.

WSL in NAT mode cannot open a port of Windows while the firewall blocks it, but Windows reaches
the distribution's localhost.  A small tunnel therefore exposes a Windows loopback port inside
WSL: Windows keeps connections open to a relay in WSL, and every connection a Linux program
opens to the relay's public port rides on one of them.  The firewall stays as it is."""
import argparse
import json
import os
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# ---- the WSL side: the relay of the tunnel and a file helper (written to a file, run by python3)
WSL_SIDE = r'''
import hashlib, json, os, socket, sys, threading, queue

def pipe(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d:
                break
            b.sendall(d)
    except OSError:
        pass
    for s in (b,):
        try:
            s.shutdown(socket.SHUT_WR)
        except OSError:
            pass

def relay(pub, ctl):
    pool = queue.Queue()
    def take_ctl():
        l = socket.socket(); l.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        l.bind(("127.0.0.1", ctl)); l.listen(64)
        while True:
            c, _ = l.accept()
            pool.put(c)
    threading.Thread(target=take_ctl, daemon=True).start()
    l = socket.socket(); l.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    l.bind(("127.0.0.1", pub)); l.listen(64)
    print("relay ready", flush=True)
    while True:
        c, _ = l.accept()
        w = pool.get()
        w.sendall(b"\x01")
        threading.Thread(target=pipe, args=(c, w), daemon=True).start()
        threading.Thread(target=pipe, args=(w, c), daemon=True).start()

def tree(root):
    out = {}
    for d, dirs, files in os.walk(root):
        for f in files:
            p = os.path.join(d, f)
            if not os.path.islink(p):
                out[os.path.relpath(p, root)] = hashlib.sha256(open(p, "rb").read()).hexdigest()
    return out

cmd = sys.argv[1]
if cmd == "relay":
    relay(int(sys.argv[2]), int(sys.argv[3]))
elif cmd == "mk":
    base = sys.argv[2]
    os.makedirs(base + "/src/sub/tief", exist_ok=True)
    os.makedirs(base + "/restore", exist_ok=True)
    for i in range(25):
        open(f"{base}/src/datei{i}.txt", "w", encoding="utf-8").write(f"Linux {i} äöü " * (i * 90 + 1))
    open(base + "/src/sub/tief/gross.bin", "wb").write(os.urandom(9 << 20))
    os.symlink("datei1.txt", base + "/src/verweis")
    os.chmod(base + "/src/datei2.txt", 0o640)
elif cmd == "tree":
    print(json.dumps(tree(sys.argv[2])))
elif cmd == "write":
    open(sys.argv[2], "w", encoding="utf-8").write(sys.argv[3])
elif cmd == "read":
    sys.stdout.write(open(sys.argv[2], encoding="utf-8").read())
elif cmd == "mode":
    print(oct(os.lstat(sys.argv[2]).st_mode & 0o7777))
elif cmd == "link":
    print(os.readlink(sys.argv[2]) if os.path.islink(sys.argv[2]) else "")
elif cmd == "free":
    s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])
'''


def win_to_wsl(p):
    p = str(p)
    return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/")


class Api:
    def __init__(self, url, cafile):
        self.url, self.token = url, None
        self.ctx = ssl.create_default_context(cafile=str(cafile))

    def call(self, method, path, body=None, expect=None):
        headers = {"Content-Type": "application/json"}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, method=method, headers=headers, data=data)
        try:
            r = urllib.request.urlopen(req, timeout=60, context=self.ctx)
        except urllib.error.HTTPError as e:
            r = e
        raw = r.read()
        out = json.loads(raw) if raw else None
        if expect is not None and r.status != expect:
            raise AssertionError(f"{method} {path}: {r.status} (expected {expect}): {raw[:400]!r}")
        return r.status, out

    def ok(self, method, path, body=None, expect=200):
        return self.call(method, path, body, expect)[1]

    def wait_op(self, op_id, timeout=240):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            o = self.ok("GET", "/api/operations/" + op_id)
            if o["status"] not in ("queued", "running", "cancelling"):
                if o["status"] != "succeeded":
                    raise AssertionError(f"operation {o['title']}: {o}")
                return o
            time.sleep(0.2)
        raise AssertionError("operation timed out")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for(cond, what, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            v = cond()
            if v:
                return v
        except OSError:
            pass
        time.sleep(0.2)
    raise AssertionError("timed out: " + what)


class Tunnel:
    """a Windows loopback port, reachable inside WSL at 127.0.0.1:pub"""

    def __init__(self, wsl, target, pub, ctl):
        self.target, self.ctl, self.stop = target, ctl, False
        # no probe connection: the relay would hand it out as a working slot
        self.proc = wsl.popen(["relay", str(pub), str(ctl)], log=subprocess.PIPE)
        if b"relay ready" not in self.proc.stdout.readline():
            raise AssertionError("the WSL relay did not start")
        for _ in range(8):
            threading.Thread(target=self.slot, daemon=True).start()

    def slot(self):
        while not self.stop:
            try:
                w = socket.create_connection(("127.0.0.1", self.ctl), timeout=5)
                w.settimeout(None)
                if w.recv(1) != b"\x01":
                    w.close()
                    continue
            except OSError:
                time.sleep(0.2)
                continue
            threading.Thread(target=self.slot, daemon=True).start()  # a new idle slot
            try:
                t = socket.create_connection(("127.0.0.1", self.target), timeout=5)
            except OSError:
                w.close()
                return
            threading.Thread(target=pipe, args=(w, t), daemon=True).start()
            pipe(t, w)
            return

    def close(self):
        self.stop = True
        self.proc.kill()


def pipe(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d:
                break
            b.sendall(d)
    except OSError:
        pass
    try:
        b.shutdown(socket.SHUT_WR)
    except OSError:
        pass


class Wsl:
    def __init__(self, distro, side_file):
        self.distro, self.side = distro, win_to_wsl(side_file)

    def argv(self, args):
        return ["wsl.exe", "-d", self.distro, "--"] + args

    def run(self, args, ok=True):
        r = subprocess.run(self.argv(args), capture_output=True, text=True, encoding="utf-8", timeout=120)
        if ok and r.returncode != 0:
            raise AssertionError(f"wsl {args[:2]}: {r.returncode}\n{r.stdout}\n{r.stderr}")
        return r

    def side_run(self, args):
        return self.run(["python3", self.side] + args).stdout

    def popen(self, args, log=None):
        return subprocess.Popen(self.argv(["python3", self.side] + args), stdout=log or subprocess.DEVNULL, stderr=subprocess.STDOUT)

    def free_port(self):
        while True:
            p = int(self.side_run(["free"]).strip())
            try:  # free on Windows too (the forwarding takes it there)
                with socket.socket() as s:
                    s.bind(("127.0.0.1", p))
                return p
            except OSError:
                pass


def setup(api, token):
    pw = "correct horse battery staple"
    api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": pw, "server_name": "cross"}, 201)
    api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": pw})["token"]


def job_body(name, host, sources, repo):
    return {"name": name, "description": "", "kind": "file", "enabled": True, "host_id": host, "sources": sources, "excludes": [],
            "repository_id": repo, "schedule": {"cron": "", "tz": "UTC", "enabled": False},
            "retention": {"points": 5, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": True,
            "passphrase": "eine lange Passphrase", "key_hint": "", "compression": "lz4", "active_full_days": 0,
            "pre_script": "", "post_script": "", "source_job": ""}


def win_tree(root):
    import hashlib
    out = {}
    for p in sorted(Path(root).rglob("*")):
        if not p.is_symlink() and p.is_file():
            out[str(p.relative_to(root)).replace("\\", "/")] = hashlib.sha256(p.read_bytes()).hexdigest()
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--distro", default="Ubuntu-24.04")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-cross-"))
    side = tmp / "wsl_side.py"
    side.write_text(WSL_SIDE, encoding="utf-8", newline="\n")
    wsl = Wsl(a.distro, side)
    base = "/tmp/" + tmp.name
    lin_bin = win_to_wsl(ROOT / "build" / "linux")
    win_bin = ROOT / "build" / "win"
    procs, tunnels, checks = [], [], 0
    logs = {}

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1
        print("  ok", what)

    def log(name):
        logs[name] = open(tmp / f"{name}.log", "w+", encoding="utf-8")
        return logs[name]

    try:
        # ================================================ Windows server, Linux agent
        print("Windows-Server, Linux-Agent")
        port = free_port()
        procs.append(subprocess.Popen([str(win_bin / "bc-server.exe"), "--data", str(tmp / "server"), "--listen", f"127.0.0.1:{port}", "--tls"], stdout=log("win-server"), stderr=subprocess.STDOUT))
        ca = tmp / "server" / "pki" / "ca.pem"
        wait_for(lambda: ca.exists() and (tmp / "server" / "setup.token").exists(), "server start")
        api = Api(f"https://127.0.0.1:{port}", ca)
        wait_for(lambda: api.call("GET", "/health")[0] == 200, "health")
        setup(api, (tmp / "server" / "setup.token").read_text().strip())
        q, ctl, aport = wsl.free_port(), wsl.free_port(), wsl.free_port()
        tunnels.append(Tunnel(wsl, port, q, ctl))
        wsl.side_run(["mk", base])
        pc = api.ok("POST", "/api/hosts/pairing", {}, 201)
        agent_args = ["--data", base + "/agent"]
        r = wsl.run([lin_bin + "/bc-agent", "pair", "--server", f"https://127.0.0.1:{q}", "--code", pc["code"], "--fingerprint", pc["ca_fingerprint"],
                     "--name", "linux-agent", "--listen", f"127.0.0.1:{aport}", "--address", f"127.0.0.1:{aport}",
                     "--root", base + "/src", "--restore-root", base + "/restore", *agent_args])
        check("Gekoppelt" in r.stdout, "Linux agent paired with the Windows server (through the tunnel)")
        procs.append(subprocess.Popen(wsl.argv([lin_bin + "/bc-agent", "run", *agent_args]), stdout=log("lin-agent"), stderr=subprocess.STDOUT))
        host = wait_for(lambda: [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "agent" and h["version"]], "agent host")[0]
        check(host["os"] == "linux", f"agent reports Linux: {host['os']}")
        wait_for(lambda: "Verbunden" in (tmp / "lin-agent.log").read_text(encoding="utf-8", errors="replace"), "heartbeat")
        check(True, "heartbeat Linux agent -> Windows server")
        lst = api.ok("GET", f"/api/hosts/{host['id']}/browse?path=" + urllib.request.quote(base + "/src"))
        check({"sub", "datei0.txt", "verweis"} <= {e["name"] for e in lst["entries"]}, "browse the Linux agent")
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        job = api.ok("POST", "/api/jobs", job_body("Linux", host["id"], [base + "/src"], repo["id"]), 201)
        api.wait_op(api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": True}, 202)["operation_id"])
        p = api.ok("GET", f"/api/jobs/{job['id']}/points")[0]
        check(p["files"] == 26, f"backup of the Linux agent on Windows: {p['files']} files")
        dest = base + "/restore/r1"
        api.wait_op(api.ok("POST", f"/api/points/{p['id']}/restore", {"items": [], "original": False, "host_id": host["id"], "target": dest, "conflict": "skip"}, 202)["operation_id"])
        want = json.loads(wsl.side_run(["tree", base + "/src"]))
        got = json.loads(wsl.side_run(["tree", dest + base + "/src"]))
        check(got == want, f"restored onto Linux byte for byte ({len(got)} files) {sorted(set(got) ^ set(want))}")
        check(wsl.side_run(["mode", dest + base + "/src/datei2.txt"]).strip() == "0o640", "Unix permissions restored")
        check(wsl.side_run(["link", dest + base + "/src/verweis"]).strip() == "datei1.txt", "symbolic link restored as a link")
        wsl.side_run(["write", base + "/src/datei3.txt", "kaputt"])
        api.wait_op(api.ok("POST", f"/api/points/{p['id']}/restore", {"items": [base + "/src/datei3.txt"], "original": True, "host_id": host["id"], "target": "", "conflict": "overwrite"}, 202)["operation_id"])
        check(wsl.side_run(["read", base + "/src/datei3.txt"]) == "Linux 3 äöü " * 271, "original place on Linux")

        # ================================================ Linux server, Windows agent
        print("Linux-Server, Windows-Agent")
        lport = wsl.free_port()
        procs.append(subprocess.Popen(wsl.argv([lin_bin + "/bc-server", "--data", base + "/server", "--listen", f"127.0.0.1:{lport}", "--tls"]), stdout=log("lin-server"), stderr=subprocess.STDOUT))
        wait_for(lambda: wsl.run(["test", "-f", base + "/server/setup.token"], ok=False).returncode == 0, "Linux server start")
        lca = tmp / "lin-ca.pem"
        lca.write_text(wsl.run(["cat", base + "/server/pki/ca.pem"]).stdout, encoding="utf-8")
        lapi = Api(f"https://127.0.0.1:{lport}", lca)
        wait_for(lambda: lapi.call("GET", "/health")[0] == 200, "Linux server health")
        setup(lapi, wsl.run(["cat", base + "/server/setup.token"]).stdout.strip())
        wa = free_port()
        a2, ctl2 = wsl.free_port(), wsl.free_port()
        tunnels.append(Tunnel(wsl, wa, a2, ctl2))
        wsrc, wdst = tmp / "wsrc", tmp / "wrestore"
        (wsrc / "Unterordner").mkdir(parents=True)
        wdst.mkdir()
        for i in range(20):
            (wsrc / f"Datei {i}.txt").write_text(f"Windows {i} ß " * (i * 80 + 1), encoding="utf-8")
        (wsrc / "Unterordner" / "groß.bin").write_bytes(os.urandom(9 << 20))
        pc = lapi.ok("POST", "/api/hosts/pairing", {}, 201)
        wagent = [str(win_bin / "bc-agent.exe")]
        wargs = ["--data", str(tmp / "wagent")]
        r = subprocess.run(wagent + ["pair", "--server", f"https://127.0.0.1:{lport}", "--code", pc["code"], "--fingerprint", pc["ca_fingerprint"], "--name", "windows-agent",
                                     "--listen", f"127.0.0.1:{wa}", "--address", f"127.0.0.1:{a2}", "--root", str(wsrc), "--restore-root", str(wdst), *wargs],
                           capture_output=True, text=True, encoding="utf-8", timeout=60)
        check(r.returncode == 0 and "Gekoppelt" in r.stdout, f"Windows agent paired with the Linux server: {r.stdout}{r.stderr}")
        procs.append(subprocess.Popen(wagent + ["run", *wargs], stdout=log("win-agent"), stderr=subprocess.STDOUT))
        wait_for(lambda: "Verbunden" in (tmp / "win-agent.log").read_text(encoding="utf-8", errors="replace"), "Windows heartbeat")
        check(True, "heartbeat Windows agent -> Linux server")
        whost = [h for h in lapi.ok("GET", "/api/hosts") if h["kind"] == "agent"][0]
        check(whost["os"] == "windows", f"agent reports Windows: {whost['os']}")
        lst = lapi.ok("GET", f"/api/hosts/{whost['id']}/browse?path=" + urllib.request.quote(str(wsrc)))
        check({"Unterordner", "Datei 0.txt"} <= {e["name"] for e in lst["entries"]}, "browse the Windows agent from Linux")
        lrepo = lapi.ok("POST", "/api/repositories", {"name": "R", "path": base + "/repo", "create": True, "immutable_days": 0}, 201)
        ljob = lapi.ok("POST", "/api/jobs", job_body("Windows", whost["id"], [str(wsrc)], lrepo["id"]), 201)
        lapi.wait_op(lapi.ok("POST", f"/api/jobs/{ljob['id']}/run", {"full": True}, 202)["operation_id"])
        lp = lapi.ok("GET", f"/api/jobs/{ljob['id']}/points")[0]
        check(lp["files"] == 21, f"backup of the Windows agent on Linux: {lp['files']} files")
        wd = wdst / "r1"
        lapi.wait_op(lapi.ok("POST", f"/api/points/{lp['id']}/restore", {"items": [], "original": False, "host_id": whost["id"], "target": str(wd), "conflict": "skip"}, 202)["operation_id"])
        rel = str(wsrc).replace(":", "")
        got, want = win_tree(wd / rel), win_tree(wsrc)
        check(got == want and len(got) == 21, f"restored onto Windows byte for byte (Windows paths from a Linux server) {sorted(set(got) ^ set(want))}")
        check(abs(os.stat(wd / rel / "Datei 7.txt").st_mtime - os.stat(wsrc / "Datei 7.txt").st_mtime) < 0.01, "times restored on Windows")
        (wsrc / "Datei 4.txt").write_text("kaputt", encoding="utf-8")
        lapi.wait_op(lapi.ok("POST", f"/api/points/{lp['id']}/restore", {"items": [str(wsrc / "Datei 4.txt")], "original": True, "host_id": whost["id"], "target": "", "conflict": "overwrite"}, 202)["operation_id"])
        check((wsrc / "Datei 4.txt").read_text(encoding="utf-8") == "Windows 4 ß " * 321, "original place on Windows")
        print(f"bc_cross: {checks} checks passed")
    except Exception:
        for name, f in logs.items():
            f.flush()
            print(f"---- {name}\n" + (tmp / f"{name}.log").read_text(encoding="utf-8", errors="replace")[-2500:], file=sys.stderr)
        raise
    finally:
        for t in tunnels:
            t.close()
        for p in procs:
            p.kill()
            p.wait()
        # the Linux programs outlive a killed wsl.exe
        subprocess.run(["wsl.exe", "-d", a.distro, "--", "pkill", "-f", base], capture_output=True)
        subprocess.run(["wsl.exe", "-d", a.distro, "--", "pkill", "-f", "wsl_side.py relay"], capture_output=True)
        for f in logs.values():
            f.close()
        if a.keep:
            print("data kept in", tmp, "and", base)
        else:
            subprocess.run(["wsl.exe", "-d", a.distro, "--", "rm", "-rf", base], capture_output=True)
            import shutil
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
