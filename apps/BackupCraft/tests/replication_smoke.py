"""replication_smoke.py -- replicas, failover / failback and failover plans of bc-server end to end.

    python apps/BackupCraft/tests/replication_smoke.py [--server build/win/bc-server.exe] [--keep]

The server backs up a folder of itself; replicas live in its restore folder.  Checked: the
replica rules (inside a restore folder, not inside the sources, one per folder), an automatic
replica follows every new point as an exact mirror (changed, new, deleted files; a stray file
in the replica goes), failover stops the syncing, failback writes the replica's changes back to
the original (and deletes nothing there), undo drops the changes made during a failover, a
failover plan switches two replicas in order, deleting is refused while a plan uses a replica."""
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


def rel(p):
    """the folder of a source below a replica (as the server maps it)"""
    s = str(p).replace(":", "").replace("\\", "/").lstrip("/")
    return Path(s)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default=str(ROOT / "build" / ("win" if os.name == "nt" else "linux") / ("bc-server.exe" if os.name == "nt" else "bc-server")))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-repl-"))
    src = tmp / "src"
    (src / "docs" / "deep").mkdir(parents=True)
    for i in range(20):
        (src / f"f{i}.txt").write_text(f"Datei {i} " * (100 + i * 37), encoding="utf-8")
    (src / "docs" / "plan.txt").write_text("Plan A", encoding="utf-8")
    (src / "docs" / "deep" / "data.bin").write_bytes(os.urandom(200_000))
    reps = tmp / "replicas"
    reps.mkdir()
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

    def replica(rid):
        return [r for r in api.ok("GET", "/api/replicas") if r["id"] == rid][0]

    def settled(rid):
        wait_for(lambda: replica(rid)["state"] not in ("syncing", "failback"), 60)
        for o in api.ok("GET", "/api/operations"):
            if o["target_id"] == rid and o["status"] in ("queued", "running"):
                wait_op(api, o["id"])
        return replica(rid)

    def run_job():
        o = wait_op(api, api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
        return api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]

    try:
        if not wait_for(lambda: srv.poll() is None and _up(api), 15):
            raise AssertionError("server did not start")
        token = (data / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": "correct horse battery staple"}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": "correct horse battery staple"}, token=None)["token"]
        check({c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}.get("replication") is True, "replication capability")
        host = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        api.ok("PUT", "/api/hosts/" + host["id"], {"name": "", "roots": [str(src)], "restore_roots": [str(reps)]})
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        job = api.ok("POST", "/api/jobs", {"name": "Projekt", "description": "", "kind": "file", "enabled": True, "host_id": host["id"], "sources": [str(src)],
                                           "excludes": [], "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                                           "retention": {"points": 10, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False,
                                           "passphrase": "", "key_hint": "", "compression": "lz4", "active_full_days": 0, "pre_script": "",
                                           "post_script": "", "source_job": ""}, 201)
        base = {"name": "Standby 1", "job_id": job["id"], "host_id": host["id"], "target": str(reps / "r1"), "auto": True}
        for bad, fld, why in ((dict(base, target=str(tmp / "elsewhere")), "target", "outside the restore folders"),
                              (dict(base, target="relativ"), "target", "not absolute"), (dict(base, name=""), "name", "no name"),
                              (dict(base, job_id="x"), "job_id", "no job")):
            st, e = api.call("POST", "/api/replicas", bad)
            check(st in (400, 403) and e["field"] == fld, f"refused {why}: {st} {e}")
        r1 = api.ok("POST", "/api/replicas", base, 201)
        check(r1["state"] == "new", "new replica")
        st, e = api.call("POST", "/api/replicas", dict(base, name="Doppelt"))
        check(st == 409 and e["field"] == "target", "one replica per folder")
        st, e = api.call("POST", f"/api/replicas/{r1['id']}/sync", {})
        check(st == 409 and "Restore-Point" in e["message"], "no point yet")
        st, e = api.call("POST", f"/api/replicas/{r1['id']}/failover", {})
        check(st == 409, "no failover of a new replica")

        # an automatic replica follows the job
        p1 = run_job()
        r = settled(r1["id"])
        mirror = reps / "r1" / rel(src)
        check(r["state"] == "ready" and r["point_id"] == p1 and r["files"] == 22, f"synced: {r}")
        check(tree_of(mirror) == tree_of(src), "exact mirror")
        sessions = api.ok("GET", f"/api/sessions?job={job['id']}")
        check(any(s["kind"] == "replica" and s["status"] == "success" for s in sessions), "replica session")
        # changes: changed, new, deleted; a stray file in the replica goes
        (src / "f3.txt").write_text("geändert", encoding="utf-8")
        (src / "neu.txt").write_text("neu", encoding="utf-8")
        (src / "f5.txt").unlink()
        shutil.rmtree(src / "docs" / "deep")
        (mirror / "streuner.txt").write_text("gehört nicht hierher", encoding="utf-8")
        p2 = run_job()
        r = settled(r1["id"])
        check(r["point_id"] == p2 and tree_of(mirror) == tree_of(src) and not (mirror / "docs" / "deep").exists(), f"mirror follows: {r['message']}")
        check("unverändert" in r["message"] and "entfernt" in r["message"], f"summary: {r['message']}")

        # failover: no syncing; the replica is used and changed
        r = api.ok("POST", f"/api/replicas/{r1['id']}/failover", {})
        check(r["state"] == "failover" and r["failover_at"] > 0, "failover")
        st, e = api.call("POST", f"/api/replicas/{r1['id']}/sync", {})
        check(st == 409 and "Failover" in e["message"], "no sync during failover")
        (src / "f7.txt").write_text("am Original während des Failovers", encoding="utf-8")
        run_job()
        time.sleep(1)
        check(replica(r1["id"])["point_id"] == p2, "the job's point is not mirrored during failover")
        (mirror / "f1.txt").write_text("im Failover bearbeitet", encoding="utf-8")
        (mirror / "failover-neu.txt").write_text("im Failover angelegt", encoding="utf-8")
        (mirror / "f2.txt").unlink()
        # failback: the replica's state goes back, nothing is deleted at the original
        o = wait_op(api, api.ok("POST", f"/api/replicas/{r1['id']}/failback", {}, 202)["operation_id"])
        check("Failback abgeschlossen" in o["result"], f"failback: {o}")
        check((src / "f1.txt").read_text(encoding="utf-8") == "im Failover bearbeitet", "change written back")
        check((src / "failover-neu.txt").read_text(encoding="utf-8") == "im Failover angelegt", "new file written back")
        check((src / "f2.txt").exists(), "nothing deleted at the original")
        r = replica(r1["id"])
        check(r["state"] == "ready" and r["failover_at"] == 0, f"ready after failback: {r}")
        fb = [s for s in api.ok("GET", f"/api/sessions?job={job['id']}") if s["kind"] == "failback"]
        check(fb and fb[0]["status"] == "success", "failback session")

        # undo: changes made during a failover are dropped
        api.ok("POST", f"/api/replicas/{r1['id']}/failover", {})
        before = tree_of(mirror)
        (mirror / "verwerfen.txt").write_text("wird verworfen", encoding="utf-8")
        (mirror / "f4.txt").write_text("wird zurückgesetzt", encoding="utf-8")
        wait_op(api, api.ok("POST", f"/api/replicas/{r1['id']}/undo", {}, 202)["operation_id"])
        check(tree_of(mirror) == before and replica(r1["id"])["state"] == "ready", "undo restores the point")

        # a failover plan: two replicas in order
        r2 = api.ok("POST", "/api/replicas", dict(base, name="Standby 2", target=str(reps / "r2"), auto=False), 201)
        wait_op(api, api.ok("POST", f"/api/replicas/{r2['id']}/sync", {}, 202)["operation_id"])
        # the newest point is the failback's (the replica's state; the original kept f2.txt)
        check(replica(r2["id"])["state"] == "ready" and tree_of(reps / "r2" / rel(src)) == before, "manual replica synced")
        st, e = api.call("POST", "/api/failover-plans", {"name": "Notfall", "replicas": [], "delay_s": 1})
        check(st == 400 and e["field"] == "replicas", "plan needs replicas")
        plan = api.ok("POST", "/api/failover-plans", {"name": "Notfall", "replicas": [r1["id"], r2["id"]], "delay_s": 1}, 201)
        t0 = time.monotonic()
        o = wait_op(api, api.ok("POST", f"/api/failover-plans/{plan['id']}/run", {}, 202)["operation_id"])
        check(o["result"] == "2 Replikate umgeschaltet" and time.monotonic() - t0 >= 1, f"plan run: {o}")
        check(replica(r1["id"])["state"] == "failover" and replica(r2["id"])["state"] == "failover", "both failed over")
        check(api.ok("GET", "/api/failover-plans")[0]["last_status"] == "success", "plan status")
        st, _ = api.call("DELETE", f"/api/replicas/{r2['id']}")
        check(st == 409, "replica used by a plan stays")
        api.ok("DELETE", f"/api/failover-plans/{plan['id']}", expect=204)
        api.ok("DELETE", f"/api/replicas/{r2['id']}", expect=204)
        check((reps / "r2" / rel(src) / "f1.txt").exists(), "files of a removed replica stay")
        audit = {x["action"] for x in api.ok("GET", "/api/audit?limit=300")}
        check({"replica.create", "replica.failover", "replica.failback", "replica.undo", "plan.run"} <= audit, "audited")
        print(f"replication_smoke: {checks} checks passed")
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
