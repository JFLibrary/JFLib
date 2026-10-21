"""push_smoke.py -- the SSH push of bc-server on Linux (standard library only).

    python3 apps/BackupCraft/tests/push_smoke.py [--bin build/linux] [--keep]
    (from Windows: wsl.exe -d Ubuntu-24.04 -- python3 apps/BackupCraft/tests/push_smoke.py)

The SSH server is coretest --ssh-serve --shell (netio's SSH transport; commands run in sh as
this user, with a temporary HOME), so the push installs the agent below that HOME, pairs it and
starts it in the background -- nothing outside the temporary directory is touched.  Covers the
host key query (the fingerprint ssh-keygen shows), a refused wrong key, refused credentials, the
push with a stored password and with a stored Ed25519 key (an update keeps the pairing), and a
backup through the pushed agent."""
import argparse
import json
import os
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
USER, PASSWORD = "bc", "geheim-123"


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
            r = urllib.request.urlopen(req, timeout=120, context=self.ctx)
        except urllib.error.HTTPError as e:
            r = e
        raw = r.read()
        out = json.loads(raw) if raw else None
        if expect is not None and r.status != expect:
            raise AssertionError(f"{method} {path}: {r.status} (expected {expect}): {raw[:400]!r}")
        return r.status, out

    def ok(self, method, path, body=None, expect=200):
        return self.call(method, path, body, expect)[1]

    def op(self, path, body, timeout=240):
        oid = self.ok("POST", path, body, 202)["operation_id"]
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            o = self.ok("GET", "/api/operations/" + oid)
            if o["status"] not in ("queued", "running", "cancelling"):
                return o
            time.sleep(0.2)
        raise AssertionError("operation timed out: " + path)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for(cond, what, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if cond():
                return
        except OSError:
            pass
        time.sleep(0.1)
    raise AssertionError("timed out: " + what)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=str(ROOT / "build" / "linux"))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    if os.name == "nt":
        print("push_smoke runs on Linux (wsl.exe -d Ubuntu-24.04 -- python3 ...)")
        return 2
    tmp = Path(tempfile.mkdtemp(prefix="bc-push-"))
    home, src = tmp / "home", tmp / "share" / "src"
    for d in (home, src / "sub"):
        d.mkdir(parents=True)
    port, sport = free_port(), free_port()
    logs = {n: open(tmp / f"{n}.log", "w+") for n in ("server", "sshd")}
    procs, checks = [], 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1
        print("  ok", what)

    try:
        procs.append(subprocess.Popen([f"{a.bin}/bc-server", "--data", str(tmp / "server"), "--listen", f"127.0.0.1:{port}", "--tls"], stdout=logs["server"], stderr=subprocess.STDOUT))
        ca = tmp / "server" / "pki" / "ca.pem"
        wait_for(lambda: ca.exists() and (tmp / "server" / "setup.token").exists(), "server start")
        api = Api(f"https://127.0.0.1:{port}", ca)
        wait_for(lambda: api.call("GET", "/health")[0] == 200, "health")
        pw = "correct horse battery staple"
        api.ok("POST", "/api/setup", {"token": (tmp / "server" / "setup.token").read_text().strip(), "username": "admin", "password": pw}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": pw})["token"]

        # the SSH server: commands in sh, HOME in the temporary directory
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "", "-f", str(tmp / "id")], check=True)
        env = dict(os.environ, HOME=str(home))
        procs.append(subprocess.Popen([f"{a.bin}/coretest", "--ssh-serve", str(sport), "--shell", "--authorized", str(tmp / "id.pub"), "--hostkey-out", str(tmp / "host.pub")],
                                      stdout=subprocess.PIPE, stderr=logs["sshd"], env=env, text=True))
        check(procs[-1].stdout.readline().strip() == "ready", "SSH test server (sh) up")
        want_fp = subprocess.run(["ssh-keygen", "-lf", str(tmp / "host.pub")], capture_output=True, text=True).stdout.split()[1]

        fp = api.ok("POST", "/api/hosts/ssh-fingerprint", {"address": "127.0.0.1", "port": sport, "fingerprint": "", "key_type": ""})
        check(fp["fingerprint"] == want_fp and fp["key_type"] == "ssh-ed25519", f"host key query: {fp}")
        st, e = api.call("POST", "/api/hosts/ssh-fingerprint", {"address": "127.0.0.1", "port": free_port(), "fingerprint": "", "key_type": ""})
        check(st == 409 and e["field"] == "address", f"nothing listening: {st} {e}")

        pcred = api.ok("POST", "/api/credentials", {"label": "bc pw", "username": USER, "kind": "password", "secret": PASSWORD}, 201)
        kcred = api.ok("POST", "/api/credentials", {"label": "bc key", "username": USER, "kind": "ssh-key", "secret": (tmp / "id").read_text()}, 201)
        wcred = api.ok("POST", "/api/credentials", {"label": "bad", "username": USER, "kind": "password", "secret": "falsch"}, 201)
        body = {"address": "127.0.0.1", "port": sport, "name": "push-agent", "host_fingerprint": want_fp, "credential_id": pcred["id"],
                "username": "", "password": "", "roots": [str(src)]}

        o = api.op("/api/hosts/ssh-push", dict(body, host_fingerprint="SHA256:" + "A" * 43))
        check(o["status"] == "failed" and "Host-Schlüssel" in o["error"], f"a different host key stops the push: {o['error']}")
        o = api.op("/api/hosts/ssh-push", dict(body, credential_id=wcred["id"]))
        check(o["status"] == "failed" and "Anmeldung abgelehnt" in o["error"], f"refused credentials: {o['error']}")
        st, e = api.call("POST", "/api/hosts/ssh-push", dict(body, credential_id="", username="", password=""))
        check(st == 400, "credentials required")

        o = api.op("/api/hosts/ssh-push", body)
        check(o["status"] == "succeeded" and "erreicht ihn" in o["result"], f"push with a stored password: {o['result'] or o['error']}")
        agent_bin = home / ".local/share/backupcraft/agent-bin/bc-agent"
        check(agent_bin.exists() and os.access(agent_bin, os.X_OK), "agent installed below HOME")
        hosts = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "agent"]
        check(len(hosts) == 1 and hosts[0]["name"] == "push-agent" and hosts[0]["roots"] == [str(src)], f"host record: {hosts}")
        host = hosts[0]

        # a backup through the pushed agent
        for i in range(12):
            (src / f"f{i}.txt").write_text(f"push {i} " * (i * 100 + 1))
        (src / "sub" / "b.bin").write_bytes(os.urandom(2 << 20))
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        job = api.ok("POST", "/api/jobs", {"name": "Push", "description": "", "kind": "file", "enabled": True, "host_id": host["id"], "sources": [str(src)],
                                           "excludes": [], "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                                           "retention": {"points": 3, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False,
                                           "passphrase": "", "key_hint": "", "compression": "lz4", "active_full_days": 0, "pre_script": "",
                                           "post_script": "", "source_job": ""}, 201)
        o = api.op(f"/api/jobs/{job['id']}/run", {"full": True})
        check(o["status"] == "succeeded", f"backup through the pushed agent: {o['error']}")
        check(api.ok("GET", f"/api/jobs/{job['id']}/points")[0]["files"] == 13, "13 files in the point")

        # again with the key: an update, the pairing stays
        o = api.op("/api/hosts/ssh-push", dict(body, credential_id=kcred["id"]))
        check(o["status"] == "succeeded" and "aktualisiert" in o["result"], f"update with a stored Ed25519 key: {o['result'] or o['error']}")
        hosts = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "agent"]
        check(len(hosts) == 1 and hosts[0]["id"] == host["id"], "the update keeps the host")
        o = api.op(f"/api/jobs/{job['id']}/run", {"full": False})
        check(o["status"] == "succeeded", f"backup after the update: {o['error']}")
        audit = json.dumps(api.ok("GET", "/api/audit?limit=200"), ensure_ascii=False)
        check("host.ssh_push" in audit and want_fp in audit, "audit names the host key")
        print(f"push_smoke: {checks} checks passed")
        return 0
    except Exception:
        for n, f in logs.items():
            f.flush()
            print(f"---- {n}\n" + (tmp / f"{n}.log").read_text(errors="replace")[-2500:], file=sys.stderr)
        agent_log = home / ".local/share/backupcraft/agent/agent.log"
        if agent_log.exists():
            print("---- agent\n" + agent_log.read_text(errors="replace")[-2500:], file=sys.stderr)
        raise
    finally:
        for p in procs:
            p.kill()
            p.wait()
        subprocess.run(["pkill", "-f", str(home)], capture_output=True)
        for f in logs.values():
            f.close()
        if a.keep:
            print("data kept in", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
