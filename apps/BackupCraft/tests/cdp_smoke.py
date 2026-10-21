"""cdp_smoke.py -- continuous data protection of bc-server end to end (standard library only).

    python apps/BackupCraft/tests/cdp_smoke.py [--server build/win/bc-server.exe] [--keep]

A file job with CDP (RPO 15 s) on a folder of the server.  Checked: the rules (file jobs only,
RPO 15 s .. 1 day), a change makes a point by itself, several changes inside one RPO make one
point, the point holds exactly the source (restore byte for byte, the file count of folders
taken over unchanged), the run used the changed paths, excluded names start nothing, a deleted
file is gone in the next point, the CDP window keeps points the count retention would drop,
GET /api/cdp, switching CDP off stops it."""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from http_smoke import Api, free_port, tree_of, wait_op  # noqa: E402
from notify_smoke import _up, wait_for  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
RPO = 15


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default=str(ROOT / "build" / ("win" if os.name == "nt" else "linux") / ("bc-server.exe" if os.name == "nt" else "bc-server")))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-cdp-"))
    src = tmp / "src"
    for d in range(30):
        (src / f"dir{d:02}").mkdir(parents=True)
        for f in range(10):
            (src / f"dir{d:02}" / f"f{f}.txt").write_text(f"Ordner {d} Datei {f} " * 50, encoding="utf-8")
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

    def points():
        return api.ok("GET", f"/api/jobs/{job['id']}/points")

    def settle():
        for o in api.ok("GET", "/api/operations"):
            if o["target_id"] == job["id"] and o["status"] in ("queued", "running"):
                wait_op(api, o["id"])

    def new_point(before, timeout=RPO + 25):
        ok = wait_for(lambda: len(points()) > before, timeout)
        settle()
        return ok

    def restored(p, name):
        dest = tmp / "restore" / name
        wait_op(api, api.ok("POST", f"/api/points/{p['id']}/restore", {"items": [], "original": False, "target": str(dest), "conflict": "skip"}, 202)["operation_id"])
        rel = str(src).replace(":", "").replace("\\", "/").lstrip("/")
        return tree_of(dest / rel)

    try:
        if not wait_for(lambda: srv.poll() is None and _up(api), 15):
            raise AssertionError("server did not start")
        token = (data / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": "correct horse battery staple"}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": "correct horse battery staple"}, token=None)["token"]
        check({c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}.get("cdp") is True, "cdp capability")
        host = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        api.ok("PUT", "/api/hosts/" + host["id"], {"name": "", "roots": [str(src)], "restore_roots": [str(tmp / "restore")]})
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        base = {"name": "Laufend", "description": "", "kind": "file", "enabled": True, "host_id": host["id"], "sources": [str(src)],
                "excludes": ["*.tmp"], "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                "retention": {"points": 2, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False, "passphrase": "",
                "key_hint": "", "compression": "lz4", "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": "",
                "cdp": True, "cdp_rpo": RPO, "cdp_keep_hours": 24}
        st, e = api.call("POST", "/api/jobs", dict(base, cdp_rpo=5))
        check(st == 400 and e["field"] == "cdp_rpo", "RPO at least 15 s")
        st, e = api.call("POST", "/api/jobs", dict(base, kind="copy", source_job="x"))
        check(st == 400, "copy jobs have no CDP")
        job = api.ok("POST", "/api/jobs", base, 201)
        check(job["cdp"] and job["cdp_rpo"] == RPO, "CDP job")
        wait_op(api, api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
        check(len(points()) == 1, "first point by hand")
        check(wait_for(lambda: any(c["job_id"] == job["id"] and c["state"] == "watching" for c in api.ok("GET", "/api/cdp")), 10), "watching")
        # the first run after the watching started walks everything; nothing changed: no point
        time.sleep(RPO + 4)
        settle()
        check(len(points()) == 1, "no point without changes")

        # a change makes a point by itself; several changes in one RPO window make one point
        t0 = time.monotonic()
        (src / "dir07" / "f3.txt").write_text("geändert durch CDP", encoding="utf-8")
        time.sleep(0.5)
        (src / "dir12" / "neu.txt").write_text("neu", encoding="utf-8")
        (src / "dir12" / "f1.txt").write_text("auch geändert", encoding="utf-8")
        check(new_point(1), "a point after changes")
        took = time.monotonic() - t0
        check(len(points()) == 2, f"one point for one window ({len(points())})")
        p2 = points()[0]
        check(restored(p2, "p2") == tree_of(src), "the CDP point holds the source")
        check(p2["files"] == 301, f"file count with folders taken over: {p2['files']}")
        sess = [s for s in api.ok("GET", f"/api/sessions?job={job['id']}") if s["point_id"] == p2["id"] and s["kind"] == "backup"][0]
        lines = [l["msg"] for l in api.ok("GET", f"/api/sessions/{sess['id']}/log")]
        check(any("CDP:" in l and "unveränderte Ordner werden übernommen" in l for l in lines), f"changed paths used: {lines[:5]}")
        ops = [o for o in api.ok("GET", "/api/operations") if o["session_id"] == sess["id"]]
        check(ops and ops[0]["owner"] == "CDP", "started by CDP")
        print(f"   first CDP point after {took:.1f} s")
        st = [c for c in api.ok("GET", "/api/cdp") if c["job_id"] == job["id"]][0]
        check(st["points"] >= 2 and st["rpo"] == RPO and st["last_change"] > 0, f"status: {st}")

        # an excluded name starts nothing
        (src / "dir03" / "x.tmp").write_text("temporär", encoding="utf-8")
        time.sleep(RPO + 6)
        check(len(points()) == 2, "excluded change: no point")
        (src / "dir03" / "x.tmp").unlink()

        # a deleted file is gone in the next point
        (src / "dir20" / "f9.txt").unlink()
        check(new_point(2), "a point after a deletion")
        p3 = points()[0]
        check(restored(p3, "p3") == tree_of(src) and p3["files"] == 300, "the deletion is in the point")
        # the CDP window keeps all three although the count retention says two
        check(len(points()) == 3, f"CDP window keeps points ({len(points())})")

        # switched off: changes make no points
        api.ok("PUT", f"/api/jobs/{job['id']}", dict(base, cdp=False))
        time.sleep(3)
        (src / "dir01" / "f1.txt").write_text("nach dem Ausschalten", encoding="utf-8")
        time.sleep(RPO + 6)
        check(len(points()) == 3, "no CDP when switched off")
        check(all(c["job_id"] != job["id"] for c in api.ok("GET", "/api/cdp")), "not listed any more")
        print(f"cdp_smoke: {checks} checks passed")
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
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
