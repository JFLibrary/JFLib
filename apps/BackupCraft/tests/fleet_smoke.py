"""fleet_smoke.py -- one bc-server watches others (fleet) end to end (standard library only).

    python apps/BackupCraft/tests/fleet_smoke.py [--server build/win/bc-server.exe] [--keep]

Two servers: the central one (A, http on loopback) and a site (B, TLS with its own CA).
Checked: fleet tokens on B (made, listed without the secret, only good for the report, revoked),
the report needs a fleet token (not a login), A reads B's CA (probe) and pins it, A refuses a
missing or wrong fingerprint and a wrong token, A keeps B's report and its state (ok, then
critical after a failed job, unauthorized after the revocation, ok with a new token,
unreachable when B is down), the token never leaves A, the dashboard of A shows B's problems,
http only for loopback, a server watching itself over http, removal and the audit."""
import argparse
import hashlib
import json
import os
import shutil
import ssl
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from http_smoke import Api, free_port  # noqa: E402
from notify_smoke import wait_for  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
PW = "correct horse battery staple"


class TlsApi(Api):
    def __init__(self, url, cafile):
        super().__init__(url)
        self.ctx = ssl.create_default_context(cafile=str(cafile))

    def call(self, method, path, body=None, expect=None, token=...):
        headers = {"Content-Type": "application/json"}
        tok = self.token if token is ... else token
        if tok:
            headers["Authorization"] = "Bearer " + tok
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, method=method, headers=headers, data=data)
        try:
            r = urllib.request.urlopen(req, timeout=60, context=self.ctx)
        except urllib.error.HTTPError as e:
            r = e
        raw = r.read()
        out = json.loads(raw) if raw else None
        if expect is not None and r.status != expect:
            raise AssertionError(f"{method} {path}: {r.status} (expected {expect}): {raw[:300]!r}")
        return r.status, out


def pem_der(path):
    text = Path(path).read_text()
    b64 = "".join(l for l in text.splitlines() if l and not l.startswith("-----"))
    import base64
    return base64.b64decode(b64)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default=str(ROOT / "build" / ("win" if os.name == "nt" else "linux") / ("bc-server.exe" if os.name == "nt" else "bc-server")))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-fleet-"))
    pa, pb = free_port(), free_port()
    logs = {n: open(tmp / f"{n}.log", "w+") for n in "AB"}
    procs = {}

    def start(n, port, tls):
        args = [a.server, "--data", str(tmp / f"data{n}"), "--listen", f"127.0.0.1:{port}"]
        if tls:
            args.append("--tls")
        procs[n] = subprocess.Popen(args, stdout=logs[n], stderr=logs[n])

    start("A", pa, False)
    start("B", pb, True)
    A = Api(f"http://127.0.0.1:{pa}")
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    def up(api):
        try:
            api.ok("GET", "/api")
            return True
        except (OSError, AssertionError):
            return False

    def set_up(api, data, name):
        token = (data / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": PW}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": PW}, token=None)["token"]
        api.ok("PUT", "/api/settings", [{"key": "server.name", "value": name}])

    try:
        check(wait_for(lambda: up(A), 15), "A starts")
        ca_b = tmp / "dataB" / "pki" / "ca.pem"
        check(wait_for(ca_b.exists, 15), "B has its CA")
        B = TlsApi(f"https://127.0.0.1:{pb}", ca_b)
        check(wait_for(lambda: up(B), 15), "B starts")
        set_up(A, tmp / "dataA", "site-a")
        set_up(B, tmp / "dataB", "site-b")
        check({c["name"]: c["available"] for c in A.ok("GET", "/api/capabilities")}.get("fleet") is True, "fleet capability")

        # ---- B: fleet tokens
        st, e = B.call("POST", "/api/fleet/tokens", {"label": " "})
        check(st == 400 and e["field"] == "label", "a label is needed")
        made = B.ok("POST", "/api/fleet/tokens", {"label": "Zentrale A"}, 201)
        tok = made["token"]
        check(tok.startswith("bcf_") and len(tok) > 40, f"token {tok[:8]}")
        listed = B.ok("GET", "/api/fleet/tokens")
        check(len(listed) == 1 and listed[0]["label"] == "Zentrale A" and tok not in json.dumps(listed), "listed without the secret")
        st, _ = B.call("GET", "/api/fleet/report", token=None)
        check(st == 401, "report without a token")
        st, _ = B.call("GET", "/api/fleet/report")
        check(st == 401, "a login token is no fleet token")
        rep = B.ok("GET", "/api/fleet/report", token=tok)
        check(rep["server_name"] == "site-b" and rep["jobs"] == 0, f"report: {rep['server_name']}")
        st, _ = B.call("GET", "/api/jobs", token=tok)
        check(st == 401, "the fleet token reads the report only")
        check(B.ok("GET", "/api/fleet/tokens")[0]["last_used"] > 0, "use noted")

        # ---- A: probe and pin B's CA
        st, e = A.call("POST", "/api/fleet/probe", {"url": "http://10.1.2.3:7642"})
        check(st == 400 and e["field"] == "url" and "http" in e["message"], "http only on loopback")
        pr = A.ok("POST", "/api/fleet/probe", {"url": f"https://127.0.0.1:{pb}"})
        want = hashlib.sha256(pem_der(ca_b)).hexdigest()
        check(pr["tls"] and pr["fingerprint"] == want and pr["url"] == f"https://127.0.0.1:{pb}", f"probe: {pr}")
        req = {"name": "Standort B", "url": f"127.0.0.1:{pb}", "token": tok, "fingerprint": ""}
        st, e = A.call("POST", "/api/fleet/servers", dict(req, url=f"https://127.0.0.1:{pb}"))
        check(st == 400 and e["field"] == "fingerprint", "a CA must be confirmed")
        st, e = A.call("POST", "/api/fleet/servers", dict(req, url=f"https://127.0.0.1:{pb}", fingerprint="00" * 32))
        check(st == 409 and e["field"] == "fingerprint", "another CA is refused")
        st, e = A.call("POST", "/api/fleet/servers", dict(req, url=f"https://127.0.0.1:{pb}", fingerprint=pr["fingerprint_text"], token="bcf_falsch"))
        check(st == 400 and e["field"] == "token", f"a wrong token is refused: {e}")
        srv = A.ok("POST", "/api/fleet/servers", dict(req, url=f"https://127.0.0.1:{pb}", fingerprint=pr["fingerprint_text"]), 201)
        check(srv["status"] == "ok" and srv["report"]["server_name"] == "site-b" and srv["fingerprint"] == want, f"watched: {srv['status']} {srv['message']}")
        raw = json.dumps(A.ok("GET", "/api/fleet"))
        check(tok not in raw and '"token"' not in raw, "the token stays on A")
        st, e = A.call("POST", "/api/fleet/servers", dict(req, url=f"https://127.0.0.1:{pb}", fingerprint=want, name="Doppelt"))
        check(st == 409 and e["field"] == "url", "watched once")

        # ---- B gets a failed job: A sees it
        src = tmp / "src"
        src.mkdir()
        host = [h for h in B.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        B.ok("PUT", "/api/hosts/" + host["id"], {"name": "", "roots": [str(src)], "restore_roots": []})
        repo = B.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        job = B.ok("POST", "/api/jobs", {"name": "Kaputt", "description": "", "kind": "file", "enabled": True, "host_id": host["id"],
                                         "sources": [str(src / "fehlt")], "excludes": [], "repository_id": repo["id"],
                                         "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                                         "retention": {"points": 3, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False,
                                         "passphrase": "", "key_hint": "", "compression": "lz4", "active_full_days": 0, "pre_script": "",
                                         "post_script": "", "source_job": "", "cdp": False, "cdp_rpo": 60, "cdp_keep_hours": 0}, 201)
        B.op(f"/api/jobs/{job['id']}/run", {"full": False}, success=False)
        srv = A.ok("POST", f"/api/fleet/servers/{srv['id']}/check")
        check(srv["status"] == "critical" and "Kaputt" in srv["message"] and srv["report"]["fail_24h"] == 1, f"critical: {srv['status']} {srv['message']}")
        alerts = A.ok("GET", "/api/summary")["alerts"]
        check(any(x["target_kind"] == "fleet" and "Kaputt" in x["title"] for x in alerts), f"dashboard alert: {[x['title'] for x in alerts]}")

        # ---- revoked: unauthorized; a new token (the CA stays pinned)
        B.ok("DELETE", f"/api/fleet/tokens/{made['id']}", expect=204)
        srv = A.ok("POST", f"/api/fleet/servers/{srv['id']}/check")
        check(srv["status"] == "unauthorized", f"revoked: {srv['status']}")
        tok2 = B.ok("POST", "/api/fleet/tokens", {"label": "Zentrale A (neu)"}, 201)["token"]
        srv = A.ok("PUT", f"/api/fleet/servers/{srv['id']}", {"name": "Standort B", "url": f"https://127.0.0.1:{pb}", "token": tok2, "fingerprint": ""})
        check(srv["status"] == "critical" and srv["name"] == "Standort B", f"new token: {srv['status']}")

        # ---- A watches itself over http (no CA)
        own = A.ok("POST", "/api/fleet/tokens", {"label": "selbst"}, 201)["token"]
        pr = A.ok("POST", "/api/fleet/probe", {"url": f"localhost:{pa}"})
        check(not pr["tls"] and pr["url"] == f"http://localhost:{pa}", f"http probe: {pr}")
        me = A.ok("POST", "/api/fleet/servers", {"name": "Ich", "url": f"localhost:{pa}", "token": own, "fingerprint": ""}, 201)
        check(me["report"]["server_name"] == "site-a" and me["fingerprint"] == "", f"self: {me['report']['server_name']}")

        # ---- B down: unreachable
        procs["B"].terminate()
        procs["B"].wait(10)
        srv = A.ok("POST", f"/api/fleet/servers/{srv['id']}/check")
        check(srv["status"] == "unreachable" and srv["seen"] <= srv["checked"], f"down: {srv['status']} {srv['message']}")
        check(srv["report"]["server_name"] == "site-b", "the last report stays")
        alerts = A.ok("GET", "/api/summary")["alerts"]
        check(any(x["target_kind"] == "fleet" and "nicht erreichbar" in x["title"] for x in alerts), "unreachable alert")

        A.ok("DELETE", f"/api/fleet/servers/{me['id']}", expect=204)
        check([s["name"] for s in A.ok("GET", "/api/fleet")] == ["Standort B"], "removed")
        acts = {e["action"] for e in A.ok("GET", "/api/audit?limit=200")}
        check({"fleet.add", "fleet.update", "fleet.check", "fleet.remove", "fleet.token.create"} <= acts, f"audit: {sorted(acts)}")
        print(f"fleet_smoke: {checks} checks passed")
        return 0
    except AssertionError as e:
        print("FAIL", e)
        for n, f in logs.items():
            f.flush()
            print(f"--- {n}")
            print((tmp / f"{n}.log").read_text()[-2000:])
        return 1
    finally:
        for p in procs.values():
            p.terminate()
            try:
                p.wait(10)
            except subprocess.TimeoutExpired:
                p.kill()
        for f in logs.values():
            f.close()
        if a.keep:
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
