"""tape_smoke.py -- the virtual tape library of bc-server end to end (standard library only).

    python apps/BackupCraft/tests/tape_smoke.py [--server build/win/bc-server.exe] [--keep]

Small virtual tapes (40 MB) and a point of about 47 MB.  Checked: pools and blank tapes, a point
written to tape spans tapes, the tape states and their protection, the catalog (browse), a
restore of everything and of one folder straight from tape (byte for byte), verify, an
encrypted pool (nothing readable on the tape, restore works), bcctl reading the tapes without
a server (the encrypted set only with the recovery kit), a tape job that follows its
source job, a missing tape (offline, restore refused) and its return, protection against erase
and delete, erase after the protection, the tapes read by another server (rescan: the clear set
restores there, the encrypted one stays locked), a damaged block found by verify, the rules."""
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
PW = "correct horse battery staple"
SECRET = b"GEHEIMNIS-4711-auf-Band"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default=str(ROOT / "build" / ("win" if os.name == "nt" else "linux") / ("bc-server.exe" if os.name == "nt" else "bc-server")))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-tape-"))
    src = tmp / "src"
    (src / "docs").mkdir(parents=True)
    for k in range(150):
        (src / "docs" / f"brief{k:03}.txt").write_text(f"Brief {k}\n" * (20 + k), encoding="utf-8")
    (src / "docs" / "geheim.txt").write_bytes(SECRET * 40)
    (src / "gross").mkdir()
    for k in range(3):
        (src / "gross" / f"daten{k}.bin").write_bytes(os.urandom(15 << 20))
    procs, logs = {}, {}

    def start(n):
        port = free_port()
        logs[n] = open(tmp / f"{n}.log", "w+")
        procs[n] = subprocess.Popen([a.server, "--data", str(tmp / f"data{n}"), "--listen", f"127.0.0.1:{port}"], stdout=logs[n], stderr=logs[n])
        api = Api(f"http://127.0.0.1:{port}")
        if not wait_for(lambda: procs[n].poll() is None and _up(api), 15):
            raise AssertionError(f"server {n} did not start")
        token = (tmp / f"data{n}" / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": PW}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": PW}, token=None)["token"]
        return api

    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    try:
        api = start("A")
        kit = api.ok("POST", "/api/recovery/kit", {}, 201)["private_key"]
        (tmp / "kit.txt").write_text(kit + "\n", encoding="utf-8")
        lib = tmp / "dataA" / "vtl"
        check({c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}.get("tape") is True, "tape capability")
        st = api.ok("GET", "/api/tape")
        check(st["tapes"] == 0 and st["library"].endswith("vtl"), f"empty library: {st}")
        host = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        api.ok("PUT", "/api/hosts/" + host["id"], {"name": "", "roots": [str(src)], "restore_roots": [str(tmp / "restore")]})
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        base = {"name": "Dateien", "description": "", "kind": "file", "enabled": True, "host_id": host["id"], "sources": [str(src)], "excludes": [],
                "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                "retention": {"points": 5, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False, "passphrase": "", "key_hint": "",
                "compression": "lz4", "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": "", "pool_id": "",
                "cdp": False, "cdp_rpo": 60, "cdp_keep_hours": 0}
        job = api.ok("POST", "/api/jobs", base, 201)
        o = wait_op(api, api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
        p1 = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]

        # ---- pools and tapes
        st_, e = api.call("POST", "/api/tape/pools", {"name": " ", "description": "", "protect_days": 7, "encrypted": False, "append": True})
        check(st_ == 400 and e["field"] == "name", "a pool needs a name")
        weekly = api.ok("POST", "/api/tape/pools", {"name": "Wöchentlich", "description": "", "protect_days": 7, "encrypted": False, "append": True}, 201)
        vault = api.ok("POST", "/api/tape/pools", {"name": "Tresor", "description": "verschlüsselt", "protect_days": 0, "encrypted": True, "append": False}, 201)
        st_, e = api.call("POST", "/api/tape/tapes", {"count": 2, "capacity_gb": 0, "capacity_mb": 0, "prefix": "B1", "pool_id": ""})
        check(st_ == 400, "a prefix of letters, a capacity")
        made = api.ok("POST", "/api/tape/tapes", {"count": 8, "capacity_gb": 0, "capacity_mb": 40, "prefix": "bc", "pool_id": ""})
        check([t["barcode"] for t in made][:2] == ["BC0001V1", "BC0002V1"] and all(t["state"] == "blank" for t in made), f"tapes: {[t['barcode'] for t in made]}")
        check((tmp / "dataA" / "vtl" / "BC0001V1.bctape").is_file(), "a tape is a file of the library")

        # ---- a point to tape: it spans tapes
        o = wait_op(api, api.ok("POST", f"/api/points/{p1}/tape", {"pool_id": weekly["id"]}, 202)["operation_id"])
        check(o["status"] == "succeeded", f"written: {o}")
        sets = api.ok("GET", "/api/tape/sets")
        s1 = sets[0]
        point = [x for x in api.ok("GET", f"/api/jobs/{job['id']}/points") if x["id"] == p1][0]
        check(len(sets) == 1 and s1["status"] == "ok" and s1["files"] == point["files"] and not s1["encrypted"], f"set: {s1}")
        check(len(s1["tapes"]) >= 2, f"the set spans tapes: {s1['tapes']}")
        print(f"   set on {s1['tapes']} ({o['result']})")
        tapes = {t["barcode"]: t for t in api.ok("GET", "/api/tape/tapes")}
        first = tapes[s1["tapes"][0]]
        check(first["state"] == "full" and first["pool_id"] == weekly["id"] and first["protected_until"] > time.time() + 6 * 86400, f"tape: {first}")
        sess = [x for x in api.ok("GET", f"/api/sessions?job={job['id']}") if x["kind"] == "tape"]
        check(sess and sess[0]["status"] == "success", "a tape session")

        # ---- catalog
        roots = api.ok("GET", f"/api/tape/sets/{s1['id']}/browse?path=")["entries"]
        check(len(roots) == 1 and roots[0]["name"] == str(src), f"roots: {roots}")
        names = {e["name"] for e in api.ok("GET", f"/api/tape/sets/{s1['id']}/browse?path={str(src)}")["entries"]}
        check(names == {"docs", "gross"}, f"browse: {names}")

        # ---- restore everything, then one folder
        rel = str(src).replace(":", "").replace("\\", "/").lstrip("/")
        o = wait_op(api, api.ok("POST", f"/api/tape/sets/{s1['id']}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "all"), "conflict": "skip", "host_id": ""}, 202)["operation_id"])
        check(o["status"] == "succeeded" and tree_of(tmp / "restore" / "all" / rel) == tree_of(src), f"restore from tape: {o.get('result') or o.get('error')}")
        sep = "\\" if os.name == "nt" else "/"
        o = wait_op(api, api.ok("POST", f"/api/tape/sets/{s1['id']}/restore", {"items": [str(src) + sep + "docs"], "original": False, "target": str(tmp / "restore" / "one"), "conflict": "skip", "host_id": ""}, 202)["operation_id"])
        got = tree_of(tmp / "restore" / "one" / rel)
        check(o["status"] == "succeeded" and got == {k: v for k, v in tree_of(src).items() if k.startswith("docs")}, f"one folder: {sorted(got)[:3]}")
        st_, e = api.call("POST", f"/api/tape/sets/{s1['id']}/restore", {"items": [], "original": False, "target": str(tmp / "elsewhere"), "conflict": "skip", "host_id": ""})
        check(st_ == 403, "only into restore folders")

        # ---- verify
        o = wait_op(api, api.ok("POST", f"/api/tape/sets/{s1['id']}/verify", {}, 202)["operation_id"])
        check(o["status"] == "succeeded" and "geprüft" in o["result"], f"verify: {o}")

        # ---- encrypted pool
        o = wait_op(api, api.ok("POST", f"/api/points/{p1}/tape", {"pool_id": vault["id"]}, 202)["operation_id"])
        check(o["status"] == "succeeded", f"encrypted: {o}")
        s2 = [x for x in api.ok("GET", "/api/tape/sets") if x["pool_id"] == vault["id"]][0]
        check(s2["encrypted"] and not set(s2["tapes"]) & set(s1["tapes"]), f"encrypted set: {s2['tapes']}")
        raw2 = b"".join((tmp / "dataA" / "vtl" / f"{b}.bctape").read_bytes() for b in s2["tapes"])
        raw1 = b"".join((tmp / "dataA" / "vtl" / f"{b}.bctape").read_bytes() for b in s1["tapes"])
        check(SECRET in raw1 and SECRET not in raw2 and b"geheim.txt" not in raw2, "nothing readable on an encrypted tape")
        o = wait_op(api, api.ok("POST", f"/api/tape/sets/{s2['id']}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "enc"), "conflict": "skip", "host_id": ""}, 202)["operation_id"])
        check(o["status"] == "succeeded" and tree_of(tmp / "restore" / "enc" / rel) == tree_of(src), "restore of the encrypted set")

        # ---- bcctl reads the tapes without a server (parts in any order)
        ctl = str(Path(a.server).with_name("bcctl.exe" if os.name == "nt" else "bcctl"))

        def bcctl(*args):
            return subprocess.run([ctl, *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
        files1 = [str(lib / f"{b}.bctape") for b in reversed(s1["tapes"])]
        r = bcctl("tape", "list", *files1)
        check(r.returncode == 0 and s1["id"] in r.stdout and "vollständig" in r.stdout, f"bcctl tape list: {r.stdout} {r.stderr}")
        r = bcctl("tape", "extract", *files1, "--to", str(tmp / "ctl1"))
        check(r.returncode == 0 and tree_of(tmp / "ctl1" / rel) == tree_of(src), f"bcctl tape extract: {r.stdout} {r.stderr}")
        files2 = [str(lib / f"{b}.bctape") for b in s2["tapes"]]
        r = bcctl("tape", "extract", *files2, "--to", str(tmp / "ctl2"))
        check(r.returncode == 1 and "--kit" in r.stderr, f"encrypted needs the kit: {r.stderr}")
        r = bcctl("tape", "extract", *files2, "--to", str(tmp / "ctl2"), "--kit", str(tmp / "kit.txt"))
        check(r.returncode == 0 and tree_of(tmp / "ctl2" / rel) == tree_of(src), f"bcctl with the recovery kit: {r.stdout} {r.stderr}")

        # ---- a tape job follows its source job
        tj = api.ok("POST", "/api/jobs", dict(base, name="Auf Band", kind="tape", source_job=job["id"], pool_id=weekly["id"], repository_id="", sources=[], host_id="",
                                              retention={"points": 0, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}), 201)
        st_, e = api.call("POST", "/api/jobs", dict(base, name="Ohne Pool", kind="tape", source_job=job["id"], pool_id="", repository_id=""))
        check(st_ == 400 and e["field"] == "pool_id", "a tape job needs a pool")
        o = wait_op(api, api.ok("POST", f"/api/jobs/{tj['id']}/run", {}, 202)["operation_id"])
        check("schon auf Band" in o["result"], f"nothing twice: {o['result']}")
        (src / "docs" / "neu.txt").write_text("nach dem ersten Band", encoding="utf-8")
        wait_op(api, api.ok("POST", f"/api/jobs/{job['id']}/run", {"full": False}, 202)["operation_id"])
        check(wait_for(lambda: len([x for x in api.ok("GET", "/api/tape/sets") if x["job_id"] == job["id"] and x["pool_id"] == weekly["id"]]) == 2, 120),
              f"the tape job wrote the new point: {[(x['title'], x['status'], x['result'], x['error']) for x in api.ok('GET', '/api/operations')[:4]]}")
        for x in api.ok("GET", "/api/operations"):
            if x["status"] in ("queued", "running"):
                wait_op(api, x["id"])
        tj_state = api.ok("GET", f"/api/jobs/{tj['id']}")
        check(tj_state["last_status"] == "success", f"tape job state: {tj_state['last_status']}")

        # ---- a missing tape
        gone = s1["tapes"][-1]
        (lib / f"{gone}.bctape").rename(tmp / f"{gone}.bctape")
        check({t["barcode"]: t["state"] for t in api.ok("GET", "/api/tape/tapes")}[gone] == "missing", "missing tape")
        check([x for x in api.ok("GET", "/api/tape/sets") if x["id"] == s1["id"]][0]["status"] == "offline", "set offline")
        o = wait_op(api, api.ok("POST", f"/api/tape/sets/{s1['id']}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "x"), "conflict": "skip", "host_id": ""}, 202)["operation_id"], success=False)
        check(o["status"] == "failed" and gone in o["error"], f"restore needs the tape: {o.get('error')}")
        (tmp / f"{gone}.bctape").rename(lib / f"{gone}.bctape")

        # ---- protection, erase
        st_, e = api.call("POST", f"/api/tape/tapes/{s1['tapes'][0]}/erase", {})
        check(st_ == 409 and e["code"] == "immutable", "protected tape: no erase")
        st_, e = api.call("DELETE", f"/api/tape/tapes/{s1['tapes'][0]}")
        check(st_ == 409, "protected tape: no delete")
        st_, e = api.call("DELETE", f"/api/tape/pools/{weekly['id']}")
        check(st_ == 409, "a pool with written tapes stays")
        o = wait_op(api, api.ok("POST", f"/api/tape/tapes/{s2['tapes'][0]}/erase", {}, 202)["operation_id"])
        check(o["status"] == "succeeded", f"erase: {o}")
        check(not [x for x in api.ok("GET", "/api/tape/sets") if x["id"] == s2["id"]], "the erased tape's set is gone")
        check({t["barcode"]: t["state"] for t in api.ok("GET", "/api/tape/tapes")}[s2["tapes"][0]] == "blank", "blank again")

        # ---- another server reads the tapes
        B = start("B")
        hostb = [h for h in B.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        B.ok("PUT", "/api/hosts/" + hostb["id"], {"name": "", "roots": [], "restore_roots": [str(tmp / "restoreB")]})
        (tmp / "dataB" / "vtl").mkdir(parents=True, exist_ok=True)
        for b in s1["tapes"] + s2["tapes"][1:]:
            shutil.copy(lib / f"{b}.bctape", tmp / "dataB" / "vtl" / f"{b}.bctape")
        o = wait_op(B, B.ok("POST", "/api/tape/rescan", {}, 202)["operation_id"])
        check(o["status"] == "succeeded" and "neu" in o["result"], f"rescan: {o.get('result')}")
        bsets = {x["id"]: x for x in B.ok("GET", "/api/tape/sets")}
        check(s1["id"] in bsets and bsets[s1["id"]]["status"] == "ok" and bsets[s1["id"]]["files"] == s1["files"], f"imported set: {list(bsets.values())[:1]}")
        names = {e["name"] for e in B.ok("GET", f"/api/tape/sets/{s1['id']}/browse?path={str(src)}")["entries"]}
        check(names == {"docs", "gross"}, "imported catalog")
        o = wait_op(B, B.ok("POST", f"/api/tape/sets/{s1['id']}/restore", {"items": [], "original": False, "target": str(tmp / "restoreB" / "s1"), "conflict": "skip", "host_id": ""}, 202)["operation_id"])
        want = {k: v for k, v in tree_of(src).items() if k != "docs/neu.txt"}
        check(o["status"] == "succeeded" and tree_of(tmp / "restoreB" / "s1" / rel) == want, "restore on another server, without the repository")

        # ---- a damaged block
        t0 = lib / f"{s1['tapes'][0]}.bctape"
        b = bytearray(t0.read_bytes())
        b[len(b) // 2] ^= 0x55
        t0.write_bytes(bytes(b))
        o = wait_op(api, api.ok("POST", f"/api/tape/sets/{s1['id']}/verify", {}, 202)["operation_id"], success=False)
        check(o["status"] == "failed" and ("beschädigt" in o["error"] or "weichen ab" in o["error"]), f"damage found: {o.get('error')}")
        acts = {e["action"] for e in api.ok("GET", "/api/audit?limit=300")}
        check({"pool.create", "tape.add", "tape.write", "tape.restore", "tape.verify", "tape.erase"} <= acts, f"audit: {sorted(acts)}")
        print(f"tape_smoke: {checks} checks passed")
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
