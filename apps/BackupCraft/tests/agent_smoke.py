"""agent_smoke.py -- bc-server with TLS and a bc-agent on this machine (standard library only).

    python apps/BackupCraft/tests/agent_smoke.py [--bin build/win] [--keep]

Isolated: temporary data directories, free loopback ports.  Covers the HTTPS API (the server's
own CA, its name in the certificate), pairing (fingerprint check, one-time code), heartbeats,
the shared folders (set from the console, locked on the agent), browsing through the agent, a
full and an incremental backup from the agent, restores onto the agent (byte for byte, conflict
rules), the agent's rules (no read outside its folders, no link followed, no plain or foreign
TLS client), and the revocation of a removed agent."""
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


class Api:
    def __init__(self, url, cafile):
        self.url = url
        self.token = None
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

    def wait_op(self, op_id, success=True, timeout=180):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            o = self.ok("GET", "/api/operations/" + op_id)
            if o["status"] not in ("queued", "running", "cancelling"):
                if (o["status"] == "succeeded") != success:
                    raise AssertionError(f"operation {o['title']}: {o}")
                return o
            time.sleep(0.1)
        raise AssertionError("operation timed out")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def tree_of(root):
    out = {}
    for p in sorted(root.rglob("*")):
        if not p.is_symlink() and p.is_file():
            out[p.relative_to(root).as_posix()] = p.read_bytes()
    return out


def wait_for(cond, what, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        v = cond()
        if v:
            return v
        time.sleep(0.1)
    raise AssertionError("timed out: " + what)


def main():
    ap = argparse.ArgumentParser()
    plat = "win" if os.name == "nt" else "linux"
    ext = ".exe" if os.name == "nt" else ""
    ap.add_argument("--bin", default=str(ROOT / "build" / plat))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    server_exe = str(Path(a.bin) / ("bc-server" + ext))
    agent_exe = str(Path(a.bin) / ("bc-agent" + ext))
    ctl_exe = str(Path(a.bin) / ("bcctl" + ext))
    tmp = Path(tempfile.mkdtemp(prefix="bc-agent-smoke-"))
    data, adata = tmp / "server", tmp / "agent"
    src, dst, outside = tmp / "share" / "src", tmp / "share" / "restore", tmp / "outside"
    for d in (src / "sub", dst, outside):
        d.mkdir(parents=True)
    port, aport = free_port(), free_port()
    slog, alog = open(tmp / "server.log", "w+"), open(tmp / "agent.log", "w+")
    procs = []
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    def run(*args, ok=True):
        r = subprocess.run(list(args), capture_output=True, text=True, timeout=60)
        if ok and r.returncode != 0:
            raise AssertionError(f"{args[0]} {args[1]}: {r.returncode}\n{r.stdout}\n{r.stderr}")
        return r

    try:
        # ---- the server speaks TLS with its own CA
        procs.append(subprocess.Popen([server_exe, "--data", str(data), "--listen", f"127.0.0.1:{port}", "--tls"], stdout=slog, stderr=slog))
        ca = data / "pki" / "ca.pem"
        wait_for(lambda: ca.exists() and (data / "setup.token").exists(), "server start")
        api = Api(f"https://127.0.0.1:{port}", ca)
        wait_for(lambda: api.call("GET", "/health")[0] == 200, "health")
        check(True, "HTTPS with the server CA, IP in the certificate")
        loc = Api(f"https://localhost:{port}", ca)
        check(loc.call("GET", "/health")[0] == 200, "DNS name localhost in the certificate")
        try:
            urllib.request.urlopen(f"https://127.0.0.1:{port}/health", timeout=10, context=ssl.create_default_context())
            check(False, "an unknown CA must fail")
        except urllib.error.URLError as e:
            check("CERTIFICATE_VERIFY_FAILED" in str(e), f"unknown CA refused: {e}")
        ca_out = run(server_exe, "ca", "--data", str(data), ok=False)
        # the running server holds the database lock: `ca` must say so instead of corrupting it
        check(ca_out.returncode != 0 or "SHA-256" in ca_out.stdout, "ca command")

        token = (data / "setup.token").read_text().strip()
        pw = "correct horse battery staple"
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": pw, "server_name": "agent-smoke"}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": pw})["token"]

        # ---- bcctl over https: unknown CA refused, trusted by fingerprint, kept, or a CA file
        cfg_env = dict(os.environ, BC_PASSWORD=pw, **({"APPDATA": str(tmp / "cfg")} if os.name == "nt" else {"XDG_CONFIG_HOME": str(tmp / "cfg")}))
        cfg_env.pop("BC_SERVER", None)
        cfg_env.pop("BC_TOKEN", None)

        def ctl(*args):
            return subprocess.run([ctl_exe, *args], capture_output=True, text=True, encoding="utf-8", env=cfg_env, timeout=60)
        url = f"https://127.0.0.1:{port}"
        r = ctl("--server", url, "status")
        check(r.returncode == 3 and "unbekannten Zertifizierungsstelle" in r.stderr, f"bcctl: unknown CA refused: {r.returncode} {r.stderr}")
        pcfp = api.ok("POST", "/api/hosts/pairing", {}, 201)["ca_fingerprint"]
        r = ctl("--server", url, "login", "--user", "admin", "--fingerprint", "11" * 32)
        check(r.returncode == 3 and "andere Zertifizierungsstelle" in r.stderr, f"bcctl: wrong fingerprint: {r.stderr}")
        r = ctl("--server", url, "login", "--user", "admin", "--fingerprint", pcfp)
        check(r.returncode == 0 and "Angemeldet" in r.stdout, f"bcctl login over https: {r.stdout} {r.stderr}")
        r = ctl("status")
        check(r.returncode == 0 and "agent-smoke" in r.stdout, f"bcctl with the kept fingerprint: {r.stdout} {r.stderr}")
        r = ctl("--ca", str(ca), "--server", f"https://localhost:{port}", "status")
        check(r.returncode == 0, f"bcctl with --ca and the DNS name: {r.stderr}")

        # ---- pairing
        pc = api.ok("POST", "/api/hosts/pairing", {}, 201)
        check(len(pc["code"]) == 9 and pc["server_url"].startswith("https://") and len(pc["ca_fingerprint"]) == 95, f"pairing code: {pc}")
        url = f"https://127.0.0.1:{port}"
        common = ["--data", str(adata)]
        bad = run(agent_exe, "pair", "--server", url, "--code", pc["code"], "--fingerprint", "00" * 32, *common, ok=False)
        check(bad.returncode != 0 and "andere Zertifizierungsstelle" in bad.stderr, f"wrong fingerprint refused: {bad.stderr}")
        wrong = run(agent_exe, "pair", "--server", url, "--code", "AAAA-BBBB", "--fingerprint", pc["ca_fingerprint"], *common, ok=False)
        check(wrong.returncode != 0 and "Pairing-Code" in wrong.stderr, f"unknown code refused: {wrong.stderr}")
        r = run(agent_exe, "pair", "--server", url, "--code", pc["code"], "--fingerprint", pc["ca_fingerprint"], "--name", "smoke-agent",
                "--listen", f"127.0.0.1:{aport}", "--address", f"127.0.0.1:{aport}", "--root", str(src), "--restore-root", str(dst), *common)
        check("Gekoppelt" in r.stdout, r.stdout)
        again = run(agent_exe, "pair", "--server", url, "--code", pc["code"], "--fingerprint", pc["ca_fingerprint"], *common, "--force", ok=False)
        check(again.returncode != 0 and "Pairing-Code" in again.stderr, "a code works once")
        cfg = json.loads((adata / "agent.json").read_text())
        check(cfg["roots"] == [str(src)] and cfg["remote_roots"] is True, f"agent.json: {cfg}")
        st = run(agent_exe, "status", *common)
        check("smoke-agent" in st.stdout, st.stdout)

        hosts = api.ok("GET", "/api/hosts")
        agent = [h for h in hosts if h["kind"] == "agent"][0]
        check(agent["name"] == "smoke-agent" and agent["address"] == f"127.0.0.1:{aport}" and len(agent["fingerprint"]) == 64, f"host record: {agent}")

        # ---- the agent runs and reports
        procs.append(subprocess.Popen([agent_exe, "run", *common], stdout=alog, stderr=alog))
        wait_for(lambda: "Verbunden" in (tmp / "agent.log").read_text(encoding="utf-8", errors="replace"), "agent heartbeat")
        check(True, "heartbeat over mutual TLS")
        # plain and foreign clients get nothing
        try:
            with socket.create_connection(("127.0.0.1", aport), timeout=5) as s:
                s.sendall(b"GET /v1/info HTTP/1.1\r\nHost: x\r\n\r\n")
                s.settimeout(5)
                got = s.recv(100)
            check(not got.startswith(b"HTTP/"), "plain HTTP refused")
        except OSError:
            check(True, "plain HTTP refused")
        try:
            c = ssl.create_default_context(cafile=str(ca))
            c.check_hostname = False
            with socket.create_connection(("127.0.0.1", aport), timeout=5) as s, c.wrap_socket(s) as t:
                t.sendall(b"GET /v1/info HTTP/1.1\r\nHost: x\r\n\r\n")
                got = t.recv(100)
            check(not got.startswith(b"HTTP/"), "TLS without the server certificate refused")
        except (OSError, ssl.SSLError):
            check(True, "TLS without the server certificate refused")

        # ---- files on the agent
        for i in range(30):
            (src / f"f{i}.txt").write_text(f"Agent {i} " * (i * 70 + 1), encoding="utf-8")
        (src / "sub" / "big.bin").write_bytes(os.urandom(11 << 20))  # more than one read (8 MiB)
        (outside / "secret.txt").write_text("geheim", encoding="utf-8")
        linked = False
        try:
            os.symlink(str(outside), str(src / "link"), target_is_directory=True)
            linked = True
        except OSError:
            pass  # Windows without the privilege for links

        h = api.ok("GET", f"/api/hosts/{agent['id']}/browse?path=" + urllib.request.quote(str(src)))
        names = {e["name"] for e in h["entries"]}
        check({"sub", "f0.txt"} <= names, f"browse through the agent: {names}")
        api.call("GET", f"/api/hosts/{agent['id']}/browse?path=" + urllib.request.quote(str(outside)), expect=403)
        if linked:
            st, e = api.call("GET", f"/api/hosts/{agent['id']}/browse?path=" + urllib.request.quote(str(src / "link")))
            check(st >= 400 and "Verknüpfung" in json.dumps(e, ensure_ascii=False), f"no browsing through a link: {st} {e}")

        # ---- folders: set from the console, locked on the agent
        api.ok("PUT", f"/api/hosts/{agent['id']}", {"name": "", "roots": [str(src), str(tmp / "share" / "more")], "restore_roots": [str(dst)]})
        cfg = json.loads((adata / "agent.json").read_text())
        check(str(tmp / "share" / "more") in cfg["roots"], "roots pushed to the agent")
        api.ok("PUT", f"/api/hosts/{agent['id']}", {"name": "", "roots": [str(src)], "restore_roots": [str(dst)]})
        run(agent_exe, "roots", "--lock", *common)
        wait_for(lambda: "übernommen" in (tmp / "agent.log").read_text(encoding="utf-8", errors="replace"), "agent reloads agent.json", 40)
        st, e = api.call("PUT", f"/api/hosts/{agent['id']}", {"name": "", "roots": [str(outside)], "restore_roots": []})
        check(st == 409 and "gesperrt" in e["message"], f"locked folders: {st} {e}")
        run(agent_exe, "roots", "--unlock", *common)

        # ---- a backup from the agent
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        jobbody = {"name": "Agent", "description": "", "kind": "file", "enabled": True, "host_id": agent["id"],
                   "sources": [str(src)], "excludes": [], "repository_id": repo["id"],
                   "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                   "retention": {"points": 5, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0},
                   "encrypted": True, "passphrase": "eine lange Passphrase", "key_hint": "", "compression": "lz4",
                   "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": ""}
        job = api.ok("POST", "/api/jobs", jobbody, 201)
        api.wait_op(api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": True}, 202)["operation_id"])
        p1 = api.ok("GET", f"/api/jobs/{job['id']}/points")[0]
        check(p1["files"] == 31 and p1["full"], f"first point from the agent: {p1}")
        (src / "f3.txt").write_text("geändert", encoding="utf-8")
        api.wait_op(api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
        points = api.ok("GET", f"/api/jobs/{job['id']}/points")
        p2 = [p for p in points if p["id"] != p1["id"]][0]
        check(not p2["full"] and p2["stored_new"] < 100_000, f"incremental from the agent: {p2}")
        if linked:
            lst = api.ok("GET", f"/api/points/{p2['id']}/browse?path=" + urllib.request.quote(str(src)))
            ln = [e for e in lst["entries"] if e["name"] == "link"]
            check(ln and ln[0]["kind"] == "link", "a link is kept as a link, not followed")

        # ---- restores onto the agent
        api.call("POST", f"/api/points/{p2['id']}/restore", {"items": [], "original": False, "host_id": agent["id"], "target": str(outside), "conflict": "skip"}, 403)
        dest = dst / "r1"
        op = api.ok("POST", f"/api/points/{p2['id']}/restore", {"items": [], "original": False, "host_id": agent["id"], "target": str(dest), "conflict": "skip"}, 202)["operation_id"]
        res = api.wait_op(op)
        rel = str(src).replace(":", "").replace("\\", "/").lstrip("/")
        got, want = tree_of(dest / rel), tree_of(src)
        want = {k: v for k, v in want.items() if not k.startswith("link/")}
        check(got == want, f"restored onto the agent: {sorted(set(got) ^ set(want))} {res}")
        check(abs(os.stat(dest / rel / "f7.txt").st_mtime - os.stat(src / "f7.txt").st_mtime) < 0.01, "times restored")
        check(not list(dest.rglob("*.bcrestore-*")), "no temporary files left")
        op = api.ok("POST", f"/api/points/{p2['id']}/restore", {"items": [str(src / "f1.txt")], "original": False, "host_id": agent["id"], "target": str(dest), "conflict": "rename"}, 202)["operation_id"]
        api.wait_op(op)
        check((dest / rel / "f1 (wiederhergestellt).txt").read_bytes() == (src / "f1.txt").read_bytes(), "rename on the agent")
        # original place, replacing a changed file
        (src / "f2.txt").write_text("kaputt", encoding="utf-8")
        op = api.ok("POST", f"/api/points/{p2['id']}/restore", {"items": [str(src / "f2.txt")], "original": True, "host_id": agent["id"], "target": "", "conflict": "overwrite"}, 202)["operation_id"]
        api.wait_op(op)
        check((src / "f2.txt").read_text(encoding="utf-8") == "Agent 2 " * 141, "original place on the agent")
        if linked:
            # a link planted in the restore folder does not lead the data out of it
            shutil.rmtree(dest)
            dest.mkdir()
            os.symlink(str(outside), str(dest / rel.split("/")[0]), target_is_directory=True)
            op = api.ok("POST", f"/api/points/{p2['id']}/restore", {"items": [], "original": False, "host_id": agent["id"], "target": str(dest), "conflict": "overwrite"}, 202)["operation_id"]
            wait_any(api, op)
            check(sorted(p.name for p in outside.iterdir()) == ["secret.txt"], "nothing written through a planted link")

        # ---- a replica on the agent: mirrored into its restore folder, a stray file removed there,
        # failback reads the replica back from the agent
        if (dest / rel.split("/")[0]).is_symlink():
            os.unlink(dest / rel.split("/")[0])
        api.wait_op(api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
        live = {k: v for k, v in tree_of(src).items() if not k.startswith("link/")}
        rep = api.ok("POST", "/api/replicas", {"name": "Agent-Replikat", "job_id": job["id"], "host_id": agent["id"], "target": str(dst / "rep"), "auto": False}, 201)
        api.wait_op(api.ok("POST", f"/api/replicas/{rep['id']}/sync", {}, 202)["operation_id"])
        mirror = dst / "rep" / rel
        check({k: v for k, v in tree_of(mirror).items() if not k.startswith("link/")} == live, "replica on the agent")
        (mirror / "streuner.txt").write_text("weg damit", encoding="utf-8")
        (mirror / "f4.txt").write_text("verbogen", encoding="utf-8")
        r = api.wait_op(api.ok("POST", f"/api/replicas/{rep['id']}/sync", {}, 202)["operation_id"])
        check(not (mirror / "streuner.txt").exists() and (mirror / "f4.txt").read_bytes() == (src / "f4.txt").read_bytes(), f"agent mirror removes and repairs: {r['result']}")
        api.ok("POST", f"/api/replicas/{rep['id']}/failover", {})
        (mirror / "f5.txt").write_text("auf dem Replikat geändert", encoding="utf-8")
        r = api.wait_op(api.ok("POST", f"/api/replicas/{rep['id']}/failback", {}, 202)["operation_id"])
        check((src / "f5.txt").read_text(encoding="utf-8") == "auf dem Replikat geändert", f"failback from the agent: {r['result']}")

        # ---- CDP on the agent: its watcher, the server's long poll
        api.ok("PUT", f"/api/jobs/{job['id']}", dict(jobbody, passphrase="", cdp=True, cdp_rpo=15, cdp_keep_hours=24))
        wait_for(lambda: any(c["job_id"] == job["id"] and c["state"] == "watching" for c in api.ok("GET", "/api/cdp")), "CDP watching the agent", 20)
        time.sleep(20)  # the first run after the start walks everything (no point: nothing changed)
        n = len(api.ok("GET", f"/api/jobs/{job['id']}/points"))
        (src / "f6.txt").write_text("CDP auf dem Agenten", encoding="utf-8")
        wait_for(lambda: len(api.ok("GET", f"/api/jobs/{job['id']}/points")) > n, "a CDP point from the agent", 60)
        newest = api.ok("GET", f"/api/jobs/{job['id']}/points")[0]
        lst = api.ok("GET", f"/api/points/{newest['id']}/browse?path=" + urllib.request.quote(str(src)))
        f6 = [e for e in lst["entries"] if e["name"] == "f6.txt"][0]
        check(f6["size"] == len("CDP auf dem Agenten".encode()), f"the CDP point has the change: {f6}")
        api.ok("PUT", f"/api/jobs/{job['id']}", dict(jobbody, passphrase="", cdp=False))
        api.call("DELETE", f"/api/replicas/{rep['id']}", expect=204)

        # ---- removal revokes the certificate
        api.call("DELETE", f"/api/jobs/{job['id']}")
        api.call("DELETE", f"/api/hosts/{agent['id']}", expect=204)
        before = (tmp / "agent.log").read_text(encoding="utf-8", errors="replace").count("Server antwortet")
        wait_for(lambda: (tmp / "agent.log").read_text(encoding="utf-8", errors="replace").count("Server antwortet") > before, "agent rejected after removal", 45)
        check(True, "removed agent rejected")
        print(f"agent_smoke: {checks} checks passed")
    except Exception:
        for name in ("server.log", "agent.log"):
            print(f"---- {name}\n" + (tmp / name).read_text(encoding="utf-8", errors="replace")[-3000:], file=sys.stderr)
        raise
    finally:
        for p in procs:
            p.kill()
            p.wait()
        slog.close()
        alog.close()
        if a.keep:
            print("data kept in", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


def wait_any(api, op_id, timeout=120):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        o = api.ok("GET", "/api/operations/" + op_id)
        if o["status"] not in ("queued", "running", "cancelling"):
            return o
        time.sleep(0.1)
    raise AssertionError("operation timed out")


if __name__ == "__main__":
    main()
