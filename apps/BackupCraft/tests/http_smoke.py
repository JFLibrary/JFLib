"""http_smoke.py -- acceptance test of bc-server over its REST API (standard library only).

    python apps/BackupCraft/tests/http_smoke.py [--server build/win/bc-server.exe] [--keep]

Isolated: a temporary data directory and a free loopback port; nothing of the user's data is
touched.  Covers the setup, logins and tokens, the role matrix, users, the lock-out after failed
logins, the audit log, settings, credentials (secrets never come back), repositories, jobs and
their checks, the recovery kit, configuration backups and their staged restore, operations; and,
when the engine is there, a backup, a restore and a verification.  The API shapes follow
bc_proto::api."""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


class Api:
    def __init__(self, url):
        self.url = url
        self.token = None

    def call(self, method, path, body=None, expect=None, token=...):
        headers = {"Content-Type": "application/json"}
        tok = self.token if token is ... else token
        if tok:
            headers["Authorization"] = "Bearer " + tok
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, method=method, headers=headers, data=data)
        try:
            r = urllib.request.urlopen(req, timeout=60)
        except urllib.error.HTTPError as e:
            r = e
        raw = r.read()
        out = json.loads(raw) if raw else None
        if expect is not None and r.status != expect:
            raise AssertionError(f"{method} {path}: {r.status} (expected {expect}): {raw[:300]!r}")
        return r.status, out

    def ok(self, method, path, body=None, expect=200, token=...):
        return self.call(method, path, body, expect, token)[1]

    def op(self, path, body=None, success=True, token=..., timeout=120):
        started = self.ok("POST", path, body or {}, 202, token)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            o = self.ok("GET", "/api/operations/" + started["operation_id"], token=token)
            if o["status"] not in ("queued", "running", "cancelling"):
                if (o["status"] == "succeeded") != success:
                    raise AssertionError(f"operation {path}: {o}")
                return o
            time.sleep(0.1)
        raise AssertionError("operation timed out: " + path)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    ap = argparse.ArgumentParser()
    exe = "bc-server.exe" if os.name == "nt" else "bc-server"
    ap.add_argument("--server", default=str(ROOT / "build" / ("win" if os.name == "nt" else "linux") / exe))
    ap.add_argument("--keep", action="store_true", help="keep the data directory")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-smoke-"))
    data = tmp / "data"
    port = free_port()
    log = open(tmp / "server.log", "w+")
    srv = subprocess.Popen([a.server, "--data", str(data), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
    api = Api(f"http://127.0.0.1:{port}")
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    try:
        deadline = time.monotonic() + 15
        while True:
            if srv.poll() is not None:
                raise AssertionError("server exited at start: " + (tmp / "server.log").read_text())
            try:
                info = api.ok("GET", "/api")
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.1)
        check(info["needs_setup"] is True, "needs setup")
        st, e = api.call("GET", "/api/jobs")
        check(st == 503 and e["code"] == "setup_required", "setup gate")

        # ---- setup
        token = (data / "setup.token").read_text().strip()
        pw = "correct horse battery staple"
        st, e = api.call("POST", "/api/setup", {"token": "x" * 24, "username": "admin", "password": pw})
        check(st == 401 and e["field"] == "token", "wrong setup token")
        st, e = api.call("POST", "/api/setup", {"token": token, "username": "admin", "password": "short"})
        check(st == 400 and e["field"] == "password", "short password")
        admin = api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": pw, "server_name": "smoke-srv"}, 201)
        check(admin["role"] == "admin", "first user is admin")
        check(not (data / "setup.token").exists(), "setup token removed")
        api.call("POST", "/api/setup", {"token": token, "username": "x", "password": pw}, 409)
        check(api.ok("GET", "/api")["name"] == "smoke-srv", "server name")

        # ---- login
        api.call("POST", "/api/auth/login", {"username": "admin", "password": "wrong password!"}, 401)
        lr = api.ok("POST", "/api/auth/login", {"username": "admin", "password": pw})
        api.token = lr["token"]
        check(lr["user"]["username"] == "admin" and lr["expires"] > time.time(), "login answer")
        check(api.ok("GET", "/api/auth/me")["role"] == "admin", "me")
        api.call("GET", "/api/jobs", token="bogus", expect=401)

        # ---- users and the role matrix
        users = {}
        for role in ("viewer", "restore", "operator"):
            u = api.ok("POST", "/api/users", {"username": f"u-{role}", "role": role, "password": pw}, 201)
            users[role] = api.ok("POST", "/api/auth/login", {"username": f"u-{role}", "password": pw}, token=None)["token"]
            check(u["role"] == role, "user role " + role)
        api.call("POST", "/api/users", {"username": "u-viewer", "role": "viewer", "password": pw}, 409)
        api.call("POST", "/api/users", {"username": "x", "role": "viewer", "password": "short"}, 400)
        matrix = [  # (method, path, body, lowest role that may)
            ("GET", "/api/jobs", None, "viewer"),
            ("GET", "/api/summary", None, "viewer"),
            ("GET", "/api/users", None, "admin"),
            ("GET", "/api/audit", None, "admin"),
            ("GET", "/api/settings", None, "admin"),
            ("GET", "/api/credentials", None, "admin"),
            ("POST", "/api/hosts/pairing", {}, "admin"),
        ]
        rank = {"viewer": 1, "restore": 2, "operator": 3, "admin": 4}
        for method, path, body, need in matrix:
            for role, tok in users.items():
                st, _ = api.call(method, path, body, token=tok)
                want = 403 if rank[role] < rank[need] else (201 if method == "POST" else 200)
                check(st == want, f"{role} {method} {path}: {st} (want {want})")
        # disabling a user ends its sessions
        vid = [u for u in api.ok("GET", "/api/users") if u["username"] == "u-viewer"][0]["id"]
        api.ok("PUT", "/api/users/" + vid, {"username": "u-viewer", "role": "viewer", "password": "", "disabled": True})
        api.call("GET", "/api/jobs", token=users["viewer"], expect=401)
        api.call("POST", "/api/auth/login", {"username": "u-viewer", "password": pw}, 401, token=None)
        # the last administrator stays
        api.call("PUT", "/api/users/" + admin["id"], {"username": "admin", "role": "viewer", "password": "", "disabled": False}, 409)
        api.call("DELETE", "/api/users/" + admin["id"], expect=409)
        # own password
        api.call("POST", "/api/users/me/password", {"old": "wrong one!!!!", "new": pw + "!"}, 403)

        # ---- lock-out after failed logins (kept per user name)
        for _ in range(5):
            api.call("POST", "/api/auth/login", {"username": "u-operator", "password": "nope nope nope"}, 401, token=None)
        st, e = api.call("POST", "/api/auth/login", {"username": "u-operator", "password": pw}, token=None)
        check(st == 429 and e["code"] == "rate_limited", "lock-out")

        # ---- settings
        s = {x["key"]: x["value"] for x in api.ok("GET", "/api/settings")}
        check(s["session.timeout_hours"] == "24", "default settings")
        api.call("PUT", "/api/settings", [{"key": "login.max_attempts", "value": "1"}], 400)
        api.call("PUT", "/api/settings", [{"key": "nope", "value": "1"}], 400)
        api.ok("PUT", "/api/settings", [{"key": "traffic.max_mbps", "value": "250"}])
        check({x["key"]: x["value"] for x in api.ok("GET", "/api/settings")}["traffic.max_mbps"] == "250", "setting stored")

        # ---- credentials: the secret never comes back
        cr = api.ok("POST", "/api/credentials", {"label": "root@nas", "username": "root", "kind": "password", "secret": "s3cr3t-value"}, 201)
        listing = json.dumps(api.ok("GET", "/api/credentials"))
        check("s3cr3t" not in listing and cr["label"] == "root@nas", "secret hidden")
        check(b"s3cr3t" not in b"".join(p.read_bytes() for p in (data / "db").iterdir() if p.suffix in (".log", ".bcs")), "secret sealed on disk")

        # ---- repositories
        repo_dir = tmp / "repo1"
        api.call("POST", "/api/repositories", {"name": "R", "path": "relative/path", "create": True}, 400)
        repo = api.ok("POST", "/api/repositories", {"name": "Primär", "path": str(repo_dir), "create": True, "immutable_days": 0}, 201)
        check(repo["capacity"] > 0 and repo["status"] in ("ok", "warning"), "repository figures")
        # the disk's real free bytes (other data on it counts), also in the dashboard's figures
        disk = shutil.disk_usage(repo_dir)
        check(0 < repo["free"] < repo["capacity"] and abs(repo["free"] - disk.free) < 1 << 30, f"free space: {repo['free']} vs {disk.free}")
        summ = api.ok("GET", "/api/summary")
        check(summ["storage_capacity"] == repo["capacity"] and 0 < summ["storage_free"] <= summ["storage_capacity"], f"summary storage: {summ['storage_free']} / {summ['storage_capacity']}")
        api.call("POST", "/api/repositories", {"name": "again", "path": str(repo_dir), "create": True}, 409)
        api.ok("PUT", "/api/repositories/" + repo["id"], {"immutable_days": 7})
        st, e = api.call("PUT", "/api/repositories/" + repo["id"], {"immutable_days": 1})
        check(st == 409 and e["code"] == "immutable", "immutability only grows")

        # ---- jobs
        hosts = api.ok("GET", "/api/hosts")
        server_host = [h for h in hosts if h["kind"] == "server"][0]
        src = tmp / "source"
        (src / "sub").mkdir(parents=True)
        for i in range(40):
            (src / f"file{i}.txt").write_text(f"Inhalt {i} " * (i * 50 + 1), encoding="utf-8")
        (src / "sub" / "big.bin").write_bytes(os.urandom(3 << 20))
        (src / "sub" / "zeros.bin").write_bytes(bytes(5 << 20))
        api.ok("PUT", "/api/hosts/" + server_host["id"], {"name": "", "roots": [str(tmp)], "restore_roots": [str(tmp / "restore")]})
        listing = api.ok("GET", "/api/hosts/" + server_host["id"] + "/browse?path=" + urllib.request.quote(str(src)))
        check(any(e["name"] == "sub" and e["kind"] == "dir" for e in listing["entries"]), "browse the server host")
        api.call("GET", "/api/hosts/" + server_host["id"] + "/browse?path=" + urllib.request.quote(str(ROOT)), expect=403)
        base = {"name": "Smoke", "description": "", "kind": "file", "enabled": True, "host_id": server_host["id"],
                "sources": [str(src)], "excludes": ["*.tmp"], "repository_id": repo["id"],
                "schedule": {"cron": "30 2 * * *", "tz": "Europe/Berlin", "enabled": True},
                "retention": {"points": 3, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0},
                "encrypted": True, "passphrase": "eine lange Passphrase", "key_hint": "", "compression": "lz4",
                "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": ""}
        for bad, fld in ((dict(base, name=""), "name"), (dict(base, sources=[str(ROOT)]), "sources"),
                         (dict(base, schedule={"cron": "61 * * * *", "tz": "UTC", "enabled": True}), "schedule"),
                         (dict(base, passphrase="kurz"), "passphrase"), (dict(base, repository_id="x"), "repository_id")):
            st, e = api.call("POST", "/api/jobs", bad)
            check(st in (400, 403) and e["field"] == fld, f"job check {fld}: {st} {e}")
        job = api.ok("POST", "/api/jobs", base, 201)
        check(job["next_run"] > time.time() and "passphrase" not in job, "job created")
        st, e = api.call("POST", "/api/jobs", dict(base, kind="volume", sources=["kein-volume"]))
        check(st == 400 and e["field"] == "sources", f"a volume job checks its volumes: {st} {e}")
        api.call("POST", "/api/jobs", dict(base, kind="hyperv"), 501)
        check(api.ok("POST", f"/api/jobs/{job['id']}/disable")["next_run"] == 0, "disable")
        check(api.ok("POST", f"/api/jobs/{job['id']}/enable")["next_run"] > 0, "enable")
        api.call("POST", f"/api/jobs/{job['id']}/run", {}, token=users["restore"], expect=403)
        api.call("DELETE", "/api/repositories/" + repo["id"], expect=409)

        engine = api.call("POST", f"/api/jobs/{job['id']}/run", {"full": False})[0] == 202
        if engine:
            run_backups(api, check, job, src, tmp, users)

        # ---- recovery kit and configuration backups
        check(api.ok("GET", "/api/recovery")["created"] == 0, "no kit yet")
        kit = api.ok("POST", "/api/recovery/kit", {}, 201)
        check(kit["private_key"].startswith("BCRK1-") and api.ok("GET", "/api/recovery")["fingerprint"] == kit["fingerprint"], "recovery kit")
        api.call("POST", "/api/config/backups", {"passphrase": "short"}, 400)
        o = api.op("/api/config/backups", {"passphrase": "config backup passphrase"})
        name = api.ok("GET", "/api/config/backups")[0]["name"]
        check(name in o["result"], "config backup listed")
        api.call("POST", "/api/config/restore", {"name": name, "passphrase": "wrong passphrase!"}, 403)
        api.ok("POST", "/api/config/restore", {"name": name, "passphrase": "config backup passphrase"})
        check((data / "config.restore").exists(), "restore staged")

        # ---- operations and the audit trail
        ops = api.ok("GET", "/api/operations")
        check(any(x["kind"] == "config.backup" and x["status"] == "succeeded" for x in ops), "operations list")
        audit = api.ok("GET", "/api/audit?limit=500")
        actions = {e["action"] for e in audit}
        for want in ("setup", "login", "user.create", "user.update", "settings.update", "credential.create",
                     "repository.create", "job.create", "job.disable", "recovery.kit", "config.backup", "config.restore"):
            check(want in actions, "audit has " + want)
        check(any(e["action"] == "login" and not e["ok"] for e in audit), "failed logins audited")
        check(all("s3cr3t" not in json.dumps(e) for e in audit), "no secrets in the audit")

        # ---- restart: the staged configuration is applied, data survives
        srv.terminate()
        srv.wait(10)
        srv = subprocess.Popen([a.server, "--data", str(data), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
        deadline = time.monotonic() + 15
        while True:
            try:
                api.ok("GET", "/health")
                break
            except OSError:
                if time.monotonic() > deadline or srv.poll() is not None:
                    raise
                time.sleep(0.1)
        check(not (data / "config.restore").exists(), "staged restore applied")
        lr = api.ok("POST", "/api/auth/login", {"username": "admin", "password": pw}, token=None)
        api.token = lr["token"]
        check(any(j["id"] == job["id"] for j in api.ok("GET", "/api/jobs")), "jobs after restart")
        print(f"http_smoke: {checks} checks passed" + ("" if engine else " (engine not available: backups skipped)"))
        return 0
    except AssertionError as e:
        print("FAIL", e)
        log.flush()
        print((tmp / "server.log").read_text()[-3000:])
        return 1
    finally:
        srv.terminate()
        try:
            srv.wait(10)
        except subprocess.TimeoutExpired:
            srv.kill()
        log.close()
        if a.keep:
            print("data kept in", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


def wait_op(api, op_id, success=True, timeout=180):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        o = api.ok("GET", "/api/operations/" + op_id)
        if o["status"] not in ("queued", "running", "cancelling"):
            if (o["status"] == "succeeded") != success:
                raise AssertionError(f"operation {o['title']}: {o}")
            return o
        time.sleep(0.1)
    raise AssertionError("operation timed out")


def tree_of(root):
    """{relative path: bytes} of every file below root"""
    out = {}
    for p in sorted(root.rglob("*")):
        if p.is_file():
            out[p.relative_to(root).as_posix()] = p.read_bytes()
    return out


def run_backups(api, check, job, src, tmp, users):
    """a full and an incremental backup, browsing, restores with their rules, verification,
    immutability, repository check and clean-up"""
    # the first run was started by the caller: wait for it (the job's last operation)
    ops = [o for o in api.ok("GET", "/api/operations") if o["target_id"] == job["id"] and o["kind"] == "job.run"]
    first = wait_op(api, ops[0]["id"])
    check("Dateien" in first["result"], "backup result text")
    points = api.ok("GET", f"/api/jobs/{job['id']}/points")
    check(len(points) == 1 and points[0]["full"] and points[0]["files"] == 42 and points[0]["signed"], f"first point: {points}")
    p1 = points[0]
    sessions = api.ok("GET", f"/api/sessions?job={job['id']}")
    check(sessions[0]["status"] == "success" and sessions[0]["point_id"] == p1["id"], f"session: {sessions[0]}")
    log = api.ok("GET", f"/api/sessions/{sessions[0]['id']}/log")
    check(any("Restore-Point" in l["msg"] for l in log), "session log")
    j = api.ok("GET", f"/api/jobs/{job['id']}")
    check(j["last_status"] == "success" and j["points"] == 1 and j["last_size"] > 8_000_000, f"job after run: {j}")

    # incremental: change one file, add one, delete one
    (src / "file3.txt").write_text("geändert " * 1000, encoding="utf-8")
    (src / "neu.txt").write_text("neue Datei", encoding="utf-8")
    (src / "file5.txt").unlink()
    op = api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"]
    st2, body2 = api.call("POST", f"/api/jobs/{job['id']}/run", {"full": False})
    if st2 == 202:
        # the incremental run was faster than this request: then it must have ended already
        check(api.ok("GET", "/api/operations/" + op)["status"] not in ("queued", "running"), "no second run while one runs")
        wait_op(api, body2["operation_id"])
    else:
        check(st2 == 409, f"runs already: {st2}")
    wait_op(api, op)
    points = api.ok("GET", f"/api/jobs/{job['id']}/points")
    p2 = max((p for p in points if p["id"] != p1["id"]), key=lambda p: p["created"])
    check(len(points) in (2, 3) and not p2["full"] and p2["files"] == 42, f"second point: {p2}")
    check(p2["stored_new"] < 200_000, f"incremental stores little: {p2['stored_new']}")

    # browse
    root = api.ok("GET", f"/api/points/{p2['id']}/browse")
    check(len(root["entries"]) == 1 and root["entries"][0]["kind"] == "dir", "browse roots")
    lst = api.ok("GET", f"/api/points/{p2['id']}/browse?path=" + urllib.request.quote(str(src)))
    names = {e["name"] for e in lst["entries"]}
    check("neu.txt" in names and "file5.txt" not in names and "sub" in names, "browse content")

    # restore the first point into an allowed folder and compare byte for byte
    dest = tmp / "restore" / "r1"
    api.call("POST", f"/api/points/{p1['id']}/restore", {"items": [], "original": False, "target": str(tmp / "elsewhere"), "conflict": "skip"}, 403)
    op = api.ok("POST", f"/api/points/{p1['id']}/restore", {"items": [], "original": False, "target": str(dest), "conflict": "skip"}, 202)["operation_id"]
    r = wait_op(api, op)
    rel = str(src).replace(":", "").replace("\\", "/").lstrip("/")
    restored = dest / rel
    original = dict(tree_of(src))
    want = dict(original)
    want.pop("neu.txt")
    want["file5.txt"] = ("Inhalt 5 " * 251).encode()
    want["file3.txt"] = ("Inhalt 3 " * 151).encode()
    got = tree_of(restored)
    check(got == want, f"restored files differ: {sorted(set(got) ^ set(want))} {r}")
    st = os.stat(restored / "file7.txt")
    check(abs(st.st_mtime - os.stat(src / "file7.txt").st_mtime) < 0.01, "modification time restored")
    # again: skip leaves everything, rename adds copies
    op = api.ok("POST", f"/api/points/{p1['id']}/restore", {"items": [str(src / "file1.txt")], "original": False, "target": str(dest), "conflict": "skip"}, 202)["operation_id"]
    check("1 übersprungen" in wait_op(api, op)["result"], "skip existing")
    op = api.ok("POST", f"/api/points/{p1['id']}/restore", {"items": [str(src / "file1.txt")], "original": False, "target": str(dest), "conflict": "rename"}, 202)["operation_id"]
    wait_op(api, op)
    check((restored / "file1 (wiederhergestellt).txt").read_bytes() == (src / "file1.txt").read_bytes(), "rename on conflict")
    # the restore role: no original place, no replacing, only allowed folders
    api.call("POST", f"/api/points/{p1['id']}/restore", {"items": [], "original": True, "target": "", "conflict": "skip"}, 403, token=users["restore"])
    api.call("POST", f"/api/points/{p1['id']}/restore", {"items": [], "original": False, "target": str(dest), "conflict": "overwrite"}, 403, token=users["restore"])
    op = api.ok("POST", f"/api/points/{p2['id']}/restore", {"items": [str(src / "neu.txt")], "original": False, "target": str(tmp / "restore" / "r2"), "conflict": "skip"}, 202, token=users["restore"])["operation_id"]
    wait_op(api, op)
    check((tmp / "restore" / "r2" / rel / "neu.txt").read_text(encoding="utf-8") == "neue Datei", "restore role into an allowed folder")
    # a link planted in the target is not followed
    if os.name != "nt":
        trap = tmp / "restore" / "trap"
        (trap / rel).parent.mkdir(parents=True, exist_ok=True)
        os.symlink(tmp / "outside", trap / rel)
        op = api.ok("POST", f"/api/points/{p1['id']}/restore", {"items": [str(src / "file1.txt")], "original": False, "target": str(trap), "conflict": "skip"}, 202)["operation_id"]
        wait_op(api, op)
        check(not (tmp / "outside").exists(), "no restore through a link")

    # verify, immutability, repository check and clean-up
    v = api.op(f"/api/points/{p2['id']}/verify")
    check("geprüft" in v["result"], "verify")
    check(api.ok("GET", f"/api/points/{p2['id']}")["verified"] > 0, "verified time")
    st, e = api.call("DELETE", f"/api/points/{p1['id']}")
    check(st == 409 and e["code"] == "immutable", "immutable point stays")
    c = api.op(f"/api/repositories/{job['repository_id']}/check", {"read_data": True})
    check("in Ordnung" in c["result"], "repository check")
    api.op(f"/api/repositories/{job['repository_id']}/gc")
    check(tree_of(src) == original, "the source is untouched")
    compression(api, check, job, tmp)
    copies(api, check, job, src, tmp)


def prose(seed, n):
    import random
    rnd = random.Random(seed)
    words = [bytes(rnd.choice(b"abcdefghijklmnopqrstuvwxyz") for _ in range(rnd.randint(2, 9))) for _ in range(2000)]
    return b" ".join(rnd.choice(words) for _ in range(n))


def compression(api, check, job, tmp):
    """the same kind of data with LZ4 and with zstd: zstd stores less, restores byte for byte"""
    stored = {}
    for k, codec in enumerate(("lz4", "zstd", "zstd-high")):
        src = tmp / f"comp-{codec}"
        src.mkdir()
        (src / "text.txt").write_bytes(prose(100 + k, 300_000))
        body = dict(api.ok("GET", f"/api/jobs/{job['id']}"), name="Kompression " + codec, sources=[str(src)], compression=codec,
                    encrypted=False, passphrase="", schedule={"cron": "", "tz": "UTC", "enabled": False})
        j = api.ok("POST", "/api/jobs", body, 201)
        check(j["compression"] == codec, "compression kept: " + codec)
        wait_op(api, api.ok("POST", f"/api/jobs/{j['id']}/run", {"full": False}, 202)["operation_id"])
        p = api.ok("GET", f"/api/jobs/{j['id']}/points")[0]
        stored[codec] = p["stored_new"]
        dest = tmp / "restore" / ("c-" + codec)
        wait_op(api, api.ok("POST", f"/api/points/{p['id']}/restore", {"items": [], "original": False, "target": str(dest), "conflict": "skip"}, 202)["operation_id"])
        rel = str(src).replace(":", "").replace("\\", "/").lstrip("/")
        check(tree_of(dest / rel) == tree_of(src), "restored byte for byte: " + codec)
        check("geprüft" in api.op(f"/api/points/{p['id']}/verify")["result"], "verify " + codec)
    check(stored["zstd"] < stored["lz4"] * 0.85, f"zstd stores less than LZ4: {stored}")
    check(stored["zstd-high"] <= stored["zstd"] * 1.02, f"zstd-high not larger: {stored}")


def copies(api, check, job, src, tmp):
    """a copy job into a second repository: the newest points, byte for byte, nothing twice,
    automatically after the source job, its own retention"""
    repo2 = api.ok("POST", "/api/repositories", {"name": "Kopien", "path": str(tmp / "repo-copies"), "create": True, "immutable_days": 0}, 201)
    body = dict(api.ok("GET", f"/api/jobs/{job['id']}"), name="Kopie nach Repo 2", kind="copy", source_job=job["id"], repository_id=repo2["id"],
                sources=[], host_id="", encrypted=False, passphrase="", compression="zstd",
                retention={"points": 2, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0},
                schedule={"cron": "", "tz": "UTC", "enabled": False})
    st, e = api.call("POST", "/api/jobs", dict(body, source_job=""))
    check(st == 400 and e["field"] == "source_job", "copy needs a source job")
    cj = api.ok("POST", "/api/jobs", body, 201)
    src_points = sorted(api.ok("GET", f"/api/jobs/{job['id']}/points"), key=lambda p: p["created"])
    r = wait_op(api, api.ok("POST", f"/api/jobs/{cj['id']}/run", {}, 202)["operation_id"])
    copies_ = api.ok("GET", f"/api/jobs/{cj['id']}/points")
    srcs = {p["id"]: p for p in src_points}
    newest_times = sorted(p["created"] for p in src_points)[-2:]
    check(len(copies_) == 2 and len({c["copied_from"] for c in copies_}) == 2 and all(c["copied_from"] in srcs for c in copies_)
          and sorted(srcs[c["copied_from"]]["created"] for c in copies_) == newest_times, f"newest two copied: {copies_} {r['result']}")
    check(all(c["repository_id"] == repo2["id"] for c in copies_), "copies in the second repository")
    cp = copies_[0]
    newest_src = srcs[cp["copied_from"]]
    check(cp["created"] == newest_src["created"] and cp["files"] == newest_src["files"] and cp["size"] == newest_src["size"], "time and size kept")
    # restore from the copy: like the source point
    for pid, name in ((newest_src["id"], "from-source"), (cp["id"], "from-copy")):
        wait_op(api, api.ok("POST", f"/api/points/{pid}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / name), "conflict": "skip"}, 202)["operation_id"])
    check(tree_of(tmp / "restore" / "from-copy") == tree_of(tmp / "restore" / "from-source") != {}, "copy restores like the source")
    check("geprüft" in api.op(f"/api/points/{cp['id']}/verify")["result"], "copy verifies")
    # nothing twice
    r = wait_op(api, api.ok("POST", f"/api/jobs/{cj['id']}/run", {}, 202)["operation_id"])
    check(r["result"].startswith("0 Restore-Points kopiert") and len(api.ok("GET", f"/api/jobs/{cj['id']}/points")) == 2, f"nothing twice: {r['result']}")
    # a new source point is copied by itself; the copy keeps two
    (src / "nach-kopie.txt").write_text("neu fuer die Kopie", encoding="utf-8")
    o = wait_op(api, api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
    newest_id = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]

    def copied():
        return any(c["copied_from"] == newest_id for c in api.ok("GET", f"/api/jobs/{cj['id']}/points"))
    deadline = time.monotonic() + 60
    while not copied() and time.monotonic() < deadline:
        time.sleep(0.2)
    check(copied(), "copied after the source job")
    for o in api.ok("GET", "/api/operations"):
        if o["target_id"] == cj["id"] and o["status"] in ("queued", "running"):
            wait_op(api, o["id"])
    check(len(api.ok("GET", f"/api/jobs/{cj['id']}/points")) == 2, "the copy's retention keeps two")
    # (a verify of the same second may list first)
    sessions = [x for x in api.ok("GET", f"/api/sessions?job={cj['id']}") if x["kind"] == "copy"]
    check(sessions and sessions[0]["status"] == "success", f"copy session: {sessions[:1]}")
    (src / "nach-kopie.txt").unlink()


if __name__ == "__main__":
    sys.exit(main())
