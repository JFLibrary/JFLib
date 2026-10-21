"""update_smoke.py -- the update channel of bc-server end to end (standard library only).

    python apps/BackupCraft/tests/update_smoke.py [--bin build/win] [--keep]

A release made with bcctl (release keygen, release manifest over two packages) is served by a
small HTTP server.  Checked: no channel / no key refused, the check finds the newer version and
the package of this platform, the dashboard hint, the download (operation) lands byte for byte
in <data>/updates, a changed manifest or another key is refused, a package whose bytes do not
match its digest is not stored, an older version is not offered, bcctl update shows the status."""
import argparse
import functools
import os
import random
import shutil
import subprocess
import sys
import tempfile
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from http_smoke import Api, free_port, wait_op  # noqa: E402
from notify_smoke import _up, wait_for  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]


class Quiet(SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=str(ROOT / "build" / ("win" if os.name == "nt" else "linux")))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    ext = ".exe" if os.name == "nt" else ""
    server, bcctl = str(Path(a.bin) / ("bc-server" + ext)), str(Path(a.bin) / ("bcctl" + ext))
    tmp = Path(tempfile.mkdtemp(prefix="bc-update-"))
    rel = tmp / "release"
    rel.mkdir()
    rnd = random.Random(5)
    win_pkg, deb_pkg = rnd.randbytes(300_000), rnd.randbytes(250_000)
    (rel / "BackupCraft_Setup.exe").write_bytes(win_pkg)
    (rel / "backupcraft_9.9.0_amd64.deb").write_bytes(deb_pkg)
    mine = win_pkg if os.name == "nt" else deb_pkg
    key = tmp / "release.key"
    pub = subprocess.run([bcctl, "release", "keygen", str(key)], capture_output=True, text=True, check=True).stdout.strip()
    other_key = tmp / "other.key"
    other_pub = subprocess.run([bcctl, "release", "keygen", str(other_key)], capture_output=True, text=True, check=True).stdout.strip()

    def sign(version, k=key, notes="Neu: Kopierjobs und Bandbibliothek."):
        subprocess.run([bcctl, "release", "manifest", "--key", str(k), "--dir", str(rel), "--version", version, "--notes", notes],
                       capture_output=True, text=True, check=True)

    sign("9.9.0")
    web = ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(rel)))
    threading.Thread(target=web.serve_forever, daemon=True).start()
    channel = f"http://127.0.0.1:{web.server_address[1]}/manifest.json"
    data = tmp / "data"
    port = free_port()
    log = open(tmp / "server.log", "w+")
    srv = subprocess.Popen([server, "--data", str(data), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
    api = Api(f"http://127.0.0.1:{port}")
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    def settings(**kv):
        api.ok("PUT", "/api/settings", [{"key": k.replace("_", ".", 1), "value": v} for k, v in kv.items()])

    try:
        if not wait_for(lambda: srv.poll() is None and _up(api), 15):
            raise AssertionError("server did not start")
        token = (data / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": "correct horse battery staple"}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": "correct horse battery staple"}, token=None)["token"]
        check({c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}.get("update") is True, "update capability")
        st = api.ok("GET", "/api/update")
        check(st["current"] == "0.1.0" and st["latest"] == "" and not st["available"], f"fresh status: {st}")
        s, e = api.call("POST", "/api/update/check", {})
        check(s == 502 and "Kein Update-Kanal" in e["message"], f"no channel: {e}")
        settings(update_url=channel)
        s, e = api.call("POST", "/api/update/check", {})
        check(s == 502 and "Schlüssel" in e["message"], f"no key: {e}")
        s, e = api.call("PUT", "/api/settings", [{"key": "update.key", "value": "abc"}])
        check(s == 400 and e["field"] == "update.key", "key checked")
        settings(update_key=pub)
        st = api.ok("POST", "/api/update/check", {})
        want_file = "BackupCraft_Setup.exe" if os.name == "nt" else "backupcraft_9.9.0_amd64.deb"
        check(st["available"] and st["latest"] == "9.9.0" and st["file"] == want_file and st["size"] == len(mine), f"found: {st}")
        check(st["notes"] == "Neu: Kopierjobs und Bandbibliothek.", "notes")
        alerts = api.ok("GET", "/api/summary")["alerts"]
        check(any(x["title"] == "BackupCraft 9.9.0 ist verfügbar" for x in alerts), f"dashboard hint: {alerts}")
        # download: byte for byte
        o = api.ok("POST", "/api/update/download", {}, 202)
        done = wait_op(api, o["operation_id"])
        st = api.ok("GET", "/api/update")
        check(st["downloaded"] and Path(st["downloaded"]).read_bytes() == mine, f"downloaded: {st} {done}")
        check(Path(st["downloaded"]).parent.parent == data / "updates", "under <data>/updates")
        # bcctl shows it
        env = dict(os.environ, BC_TOKEN=api.token)
        out = subprocess.run([bcctl, "--server", api.url, "update"], capture_output=True, text=True, encoding="utf-8", env=env).stdout
        check("Neueste:        9.9.0  (verfügbar)" in out and "Heruntergeladen:" in out, f"bcctl update: {out}")
        # a changed manifest, another key
        m = (rel / "manifest.json").read_text(encoding="utf-8")
        (rel / "manifest.json").write_text(m.replace("Bandbibliothek", "Bandbibliotek"), encoding="utf-8")
        s, e = api.call("POST", "/api/update/check", {})
        check(s == 502 and "Signatur" in e["message"], f"changed manifest: {e}")
        check(api.ok("GET", "/api/update")["error"] != "", "error kept")
        sign("9.9.1", k=other_key)
        s, e = api.call("POST", "/api/update/check", {})
        check(s == 502 and "Signatur" in e["message"], f"other key: {e}")
        settings(update_key=f"{pub} {other_pub}")
        st = api.ok("POST", "/api/update/check", {})
        check(st["latest"] == "9.9.1" and st["downloaded"] == "", "second key accepted, new version not downloaded yet")
        # bytes that do not match the digest: not stored
        target = rel / want_file
        target.write_bytes(mine[:-1] + bytes([mine[-1] ^ 1]))
        o = api.ok("POST", "/api/update/download", {}, 202)
        bad = wait_op(api, o["operation_id"], success=False)
        check("Prüfsumme" in bad["error"] and api.ok("GET", "/api/update")["downloaded"] == "", f"digest mismatch: {bad}")
        # an older version is not offered
        target.write_bytes(mine)
        sign("0.0.9")
        st = api.ok("POST", "/api/update/check", {})
        check(st["latest"] == "0.0.9" and not st["available"], f"older version: {st}")
        check(not any("ist verfügbar" in x["title"] for x in api.ok("GET", "/api/summary")["alerts"]), "no hint for an older version")
        audit = {x["action"] for x in api.ok("GET", "/api/audit?limit=200")}
        check({"update.check", "update.download"} <= audit, "audited")
        print(f"update_smoke: {checks} checks passed")
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
        web.shutdown()
        if a.keep:
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
