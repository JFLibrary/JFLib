"""bc_crash.py -- BackupCraft survives being killed in the middle of a backup.

    python tools/bc_crash.py [--rounds N] [--mb SIZE] [--target win|linux]

A server with a repository and a job over SIZE MB of random files is killed (SIGKILL /
TerminateProcess) at a random moment of a running backup, N times.  After every kill the
server must start again; its database recovers, the interrupted operation is marked so,
the session and the job no longer claim to run, a full repository check (every chunk read)
finds nothing damaged, and the next backup succeeds and restores byte for byte.
Linux runs inside WSL when started on Windows."""
import argparse
import json
import os
import random
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Api:
    def __init__(self, url):
        self.url, self.token = url, None

    def call(self, method, path, body=None):
        h = {"Content-Type": "application/json"}
        if self.token:
            h["Authorization"] = "Bearer " + self.token
        req = urllib.request.Request(self.url + path, method=method, headers=h, data=None if body is None else json.dumps(body).encode())
        try:
            r = urllib.request.urlopen(req, timeout=120)
        except urllib.error.HTTPError as e:
            r = e
        raw = r.read()
        return r.status, (json.loads(raw) if raw else None)

    def ok(self, method, path, body=None, want=(200, 201, 202, 204)):
        st, v = self.call(method, path, body)
        if st not in want:
            raise AssertionError(f"{method} {path}: {st} {v}")
        return v

    def wait(self, op, timeout=600):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            o = self.ok("GET", "/api/operations/" + op)
            if o["status"] not in ("queued", "running", "cancelling"):
                return o
            time.sleep(0.2)
        raise AssertionError("operation timed out")


def start(exe, data, port, log):
    p = subprocess.Popen([exe, "--data", str(data), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
    api = Api(f"http://127.0.0.1:{port}")
    end = time.monotonic() + 20
    while True:
        if p.poll() is not None:
            raise AssertionError("server did not start")
        try:
            api.ok("GET", "/health")
            return p, api
        except OSError:
            if time.monotonic() > end:
                raise
            time.sleep(0.1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--mb", type=int, default=300)
    ap.add_argument("--target", default="win" if os.name == "nt" else "linux")
    a = ap.parse_args()
    if a.target == "linux" and os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/bc_crash.py --rounds {a.rounds} --mb {a.mb}"]).returncode
    exe = str(ROOT / "build" / a.target / ("bc-server.exe" if a.target == "win" else "bc-server"))
    tmp = Path(tempfile.mkdtemp(prefix="bc-crash-"))
    rnd = random.Random(1)
    src = tmp / "src"
    src.mkdir()
    for i in range(a.mb // 10):
        (src / f"data{i:03}.bin").write_bytes(os.urandom(10 << 20))
    for i in range(500):
        (src / "klein" / f"{i}.txt").parent.mkdir(exist_ok=True)
        (src / "klein" / f"{i}.txt").write_text(f"Datei {i}\n" * (i % 50 + 1))
    port = free_port()
    log = open(tmp / "server.log", "w+")
    srv, api = start(exe, tmp / "data", port, log)
    pw = "crash test password"
    api.ok("POST", "/api/setup", {"token": (tmp / "data" / "setup.token").read_text().strip(), "username": "admin", "password": pw})
    api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": pw})["token"]
    host = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
    api.ok("PUT", "/api/hosts/" + host["id"], {"name": "", "roots": [str(tmp)], "restore_roots": [str(tmp / "restore")]})
    repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True})
    job = api.ok("POST", "/api/jobs", {"name": "Crash", "kind": "file", "enabled": True, "host_id": host["id"], "sources": [str(src)], "excludes": [],
                                       "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                                       "retention": {"points": 50, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False,
                                       "passphrase": "", "key_hint": "", "compression": "lz4", "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": ""})
    ok = True
    try:
        for r in range(a.rounds):
            op = api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": r % 2 == 1})["operation_id"]
            time.sleep(rnd.uniform(0.2, 3.0))
            srv.kill()
            srv.wait()
            token = api.token
            srv, api = start(exe, tmp / "data", port, log)
            api.token = token  # the login survives the restart (tokens are in the database)
            o = api.ok("GET", "/api/operations/" + op)
            sessions = api.ok("GET", f"/api/sessions?job={job['id']}")
            j = api.ok("GET", "/api/jobs/" + job["id"])
            running = [s for s in sessions if s["status"] == "running"]
            chk = api.wait(api.ok("POST", f"/api/repositories/{repo['id']}/check", {"read_data": True})["operation_id"])
            state = f"round {r + 1}: operation {o['status']}, job {j['last_status']}, check {chk['status']}: {chk.get('result') or chk.get('error')}"
            print(state)
            if o["status"] not in ("interrupted", "succeeded") or running or j["last_status"] == "running" or chk["status"] != "succeeded":
                ok = False
        # after all the kills: a backup and a byte-exact restore
        done = api.wait(api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False})["operation_id"])
        pts = sorted(api.ok("GET", f"/api/jobs/{job['id']}/points"), key=lambda p: p["created"])
        rop = api.wait(api.ok("POST", f"/api/points/{pts[-1]['id']}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "x"), "conflict": "skip"})["operation_id"])
        rel = str(src).replace(":", "").replace("\\", "/").lstrip("/")
        same = all((tmp / "restore" / "x" / rel / p.relative_to(src)).read_bytes() == p.read_bytes() for p in src.rglob("*") if p.is_file())
        gc = api.wait(api.ok("POST", f"/api/repositories/{repo['id']}/gc")["operation_id"])
        print(f"final backup {done['status']}, restore {rop['status']} ({'identical' if same else 'DIFFERENT'}), gc {gc['status']}: {gc.get('result')}")
        ok = ok and done["status"] == "succeeded" and rop["status"] == "succeeded" and same and gc["status"] == "succeeded"
    finally:
        srv.kill()
        srv.wait()
        log.close()
        if ok:
            shutil.rmtree(tmp, ignore_errors=True)
        else:
            print("kept for inspection:", tmp)
    print("bc_crash:", "all rounds consistent" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
