"""volume_smoke.py -- whole volumes block by block in bc-server (standard library only).

    python apps/BackupCraft/tests/volume_smoke.py [--server PATH] [--volume V] [--keep]

Without --volume (no rights needed): on Linux an ext4 image file is the volume: backed up,
restored byte for byte, changed in two blocks and backed up again (two blocks new); on Windows
the volume list, and that VSS without administrator rights fails with a clear message.
With --volume (run by tests/volume_admin.ps1 as administrator or tests/volume_root.sh as
root): that volume through VSS or LVM -- the session names the snapshot, the restored image
has the volume's size and its file system's signature, a second run stores few new blocks."""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from http_smoke import Api, free_port, wait_op  # noqa: E402
from notify_smoke import _up, wait_for  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
PW = "correct horse battery staple"
MB = 1 << 20


def is_admin():
    if os.name == "nt":
        import ctypes
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    return os.geteuid() == 0


def main():
    ap = argparse.ArgumentParser()
    win = os.name == "nt"
    ap.add_argument("--server", default=str(ROOT / "build" / ("win" if win else "linux") / ("bc-server.exe" if win else "bc-server")))
    ap.add_argument("--volume", default="")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-vol-"))
    port = free_port()
    log = open(tmp / "server.log", "w+")
    srv = subprocess.Popen([a.server, "--data", str(tmp / "data"), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
    api = Api(f"http://127.0.0.1:{port}")
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    def session_lines(o):
        return [l["msg"] for l in api.ok("GET", f"/api/sessions/{o['session_id']}/log")]

    try:
        if not wait_for(lambda: srv.poll() is None and _up(api), 15):
            raise AssertionError("server did not start")
        token = (tmp / "data" / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": PW}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": PW}, token=None)["token"]
        check({c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}.get("volume-backup") is True, "capability")
        host = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        vols = api.ok("GET", f"/api/hosts/{host['id']}/volumes")
        check(isinstance(vols, list) and (not win or any(v["id"] == "C:" for v in vols)), f"volumes: {[v['id'] for v in vols]}")
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        api.ok("PUT", "/api/hosts/" + host["id"], {"name": "", "roots": [], "restore_roots": [str(tmp / "restore")]})
        job = {"name": "Volume", "description": "", "kind": "volume", "enabled": True, "host_id": host["id"], "sources": [], "excludes": [],
               "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
               "retention": {"points": 5, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False, "passphrase": "", "key_hint": "",
               "compression": "lz4", "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": "", "pool_id": "",
               "cdp": False, "cdp_rpo": 60, "cdp_keep_hours": 0}
        st, e = api.call("POST", "/api/jobs", dict(job, sources=["../etc" if not win else "C:\\Windows"]))
        check(st == 400 and e["field"] == "sources", "no volume names that are not")

        def run(j):
            return wait_any(api, api.ok("POST", f"/api/jobs/{j['id']}/run", {"full": False}, 202)["operation_id"])

        if a.volume:
            # a real volume through VSS / LVM (administrator / root)
            j = api.ok("POST", "/api/jobs", dict(job, sources=[a.volume]), 201)
            o = run(j)
            lines = session_lines(o)
            check(o["status"] == "succeeded", f"backup: {o.get('error')} {lines}")
            check(any("VSS-Snapshot" in l or "LVM-Snapshot" in l for l in lines), f"through a snapshot: {lines}")
            p = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]
            wait_op(api, api.ok("POST", f"/api/points/{p}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "v"), "conflict": "skip"}, 202)["operation_id"])
            img = next((tmp / "restore" / "v").rglob("volume.raw"))
            cfg = json.loads(next((tmp / "restore" / "v").rglob("volume.json")).read_text())
            head = img.open("rb").read(2048)
            check(img.stat().st_size == cfg["size"] and cfg["snapshot"] in ("VSS", "LVM"), f"image: {cfg}")
            check(head[3:11] == b"NTFS    " or head[1080:1082] == b"\x53\xef", "the file system's signature")
            o = run(j)
            lines = session_lines(o)
            new = [l for l in lines if "Blöcken neu gespeichert" in l]
            check(o["status"] == "succeeded" and new, f"second run: {lines}")
            n, total = new[0].split(": ")[1].split(" von ")[0], new[0].split(" von ")[1].split(" ")[0]
            check(int(n) * 4 < int(total), f"few new blocks: {new[0]}")
        elif win:
            if is_admin():
                print("   (as administrator: run with --volume V: on a test volume, see volume_admin.ps1)")
            else:
                j = api.ok("POST", "/api/jobs", dict(job, sources=["C:"]), 201)
                o = run(j)
                check(o["status"] == "failed" and "Administrator" in (o.get("error") or "") + " ".join(session_lines(o)), f"VSS needs rights: {o.get('error')}")
        else:
            img = tmp / "disk.img"
            with open(img, "wb") as f:
                f.truncate(64 * MB)
            subprocess.run(["mkfs.ext4", "-F", "-q", str(img)], check=True)
            data = bytearray(img.read_bytes())
            data[20 * MB:20 * MB + 1000] = os.urandom(1000)
            img.write_bytes(data)
            j = api.ok("POST", "/api/jobs", dict(job, sources=[str(img)]), 201)
            o = run(j)
            lines = session_lines(o)
            check(o["status"] == "succeeded" and any("ohne Snapshot" in l for l in lines) and any("64 von 64 Blöcken" in l or "von 64 Blöcken" in l for l in lines), f"first run: {lines}")
            p = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]
            names = {e["name"] for e in api.ok("GET", f"/api/points/{p}/browse?path={img}")["entries"]}
            check(names == {"volume.json", "volume.raw"}, f"point: {names}")
            wait_op(api, api.ok("POST", f"/api/points/{p}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "a"), "conflict": "skip"}, 202)["operation_id"])
            got = next((tmp / "restore" / "a").rglob("volume.raw")).read_bytes()
            check(got == bytes(data), "restored byte for byte")
            cfg = json.loads(next((tmp / "restore" / "a").rglob("volume.json")).read_text())
            check(cfg["size"] == 64 * MB and cfg["block"] == MB and cfg["snapshot"] == "", f"volume.json: {cfg}")
            # two blocks change
            data[3 * MB + 5:3 * MB + 9] = b"XXXX"
            data[40 * MB:40 * MB + 4] = b"YYYY"
            img.write_bytes(data)
            o = run(j)
            lines = session_lines(o)
            check(any("2 von 64 Blöcken neu gespeichert" in l for l in lines), f"two new blocks: {[l for l in lines if 'Blöcken' in l]}")
            p2 = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]
            wait_op(api, api.ok("POST", f"/api/points/{p2}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "b"), "conflict": "skip"}, 202)["operation_id"])
            check(next((tmp / "restore" / "b").rglob("volume.raw")).read_bytes() == bytes(data), "second point byte for byte")
        print(f"volume_smoke: {checks} checks passed")
        return 0
    except AssertionError as e:
        print("FAIL", e)
        log.flush()
        print((tmp / "server.log").read_text()[-2000:])
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


def wait_any(api, op_id, timeout=3600):
    import time
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        o = api.ok("GET", "/api/operations/" + op_id)
        if o["status"] not in ("queued", "running", "cancelling"):
            return o
        time.sleep(0.2)
    raise AssertionError("operation timed out")


if __name__ == "__main__":
    sys.exit(main())
