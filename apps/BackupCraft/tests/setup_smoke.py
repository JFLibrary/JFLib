"""setup_smoke.py -- the console's setup assistant end to end, without a service.

    python apps/BackupCraft/tests/setup_smoke.py [--console build/win/BackupCraft.exe] [--keep]

The console runs a script through the assistant (a backup server on this computer, the first
administrator, a repository) with BC_SETUP_NO_SERVICE=1: the server starts as a child of the
console instead of a service (no administrator rights), on a free port (BC_SETUP_PORT).
Checked: the server's data folder, the first setup, the sign-in, the repository (listed in
the console and on disk), the job wizard opening, the console remembering the server.  The
service install itself (UAC / pkexec) is left to a manual run of the setup."""
import argparse
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
PW = "korrektes Pferd Batterie"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def ended(port):
    """nothing listens on the port (within 5 s)"""
    for _ in range(25):
        try:
            socket.create_connection(("127.0.0.1", port), 0.5).close()
        except OSError:
            return True
        time.sleep(0.2)
    return False


def main():
    win = os.name == "nt"
    ap = argparse.ArgumentParser()
    ap.add_argument("--console", default=str(ROOT / "build" / ("win" if win else "linux") / ("BackupCraft.exe" if win else "BackupCraft")))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    a.console = str(Path(a.console).resolve())
    tmp = Path(tempfile.mkdtemp(prefix="bc-setup-"))
    data, repo, cfg = tmp / "data", tmp / "repo", tmp / "cfg"
    cfg.mkdir()
    port = free_port()
    script = tmp / "setup.uis"
    q = lambda p: str(p).replace("\\", "\\\\")  # a backslash is doubled in the script's strings
    script.write_text(f"""size 1440 900
wait 50
expect "Willkommen bei BackupCraft" exists
click "Backup-Server auf diesem Computer"
click "Weiter"
wait 20
focus #0xE01014
key ctrl+a
type "{q(data)}"
click "Weiter"
wait 20
focus #0xE0101F
type "{PW}"
focus #0xE01020
type "{PW}"
wait 10
click "Weiter"
wait 20
focus #0xE0102A
key ctrl+a
type "{q(repo)}"
wait 10
click "Weiter"
wait 20
expect "Einrichten" enabled
click "Einrichten"
wait 50
sync 30000
wait 100
sync 30000
wait 100
sync 30000
wait 100
sync 30000
wait 200
expect "Neuer Sicherungsjob" exists
shot build/shots/bc/test_setup_done.png
wait 10000
click "Abbrechen"
wait 50
click "Repositories"
wait 50
sync 10000
wait 50
expect "Primär" exists
""", encoding="utf-8")
    env = dict(os.environ, BC_SETUP_NO_SERVICE="1", BC_SETUP_PORT=str(port))
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    try:
        r = subprocess.run([a.console, "--setup", "--config", str(cfg), "--script", str(script)], env=env, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=300, cwd=str(ROOT))
        out = r.stdout + r.stderr
        check(r.returncode == 0, f"the assistant's script: {out[-2000:]}")
        check((data / "pki" / "ca.pem").exists() and (data / "db").is_dir(), "the server's data folder")
        check(not (data / "setup.token").exists(), "the first setup used the token")
        check(any(repo.iterdir()), "the repository on disk")
        ini = (cfg / "console.ini").read_text(encoding="utf-8")
        check(f"127.0.0.1:{port}" in ini, f"the console remembers the server: {ini}")
        check(ended(port), "the server ended with the console")
        print(f"setup_smoke: {checks} checks passed")
        return 0
    except AssertionError as e:
        print("FAIL", e)
        return 1
    finally:
        if not win:
            subprocess.run(["pkill", "-f", f"bc-server run --data {data}"], capture_output=True)
        if a.keep:
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
