"""Android: the secret store (plat::secret over the Android Keystore) on a device or emulator
(android/tests/GallerySecretsInstrumentation.java, apps/androidGallery/secrets.rs).

    python apps/androidGallery/tests/android_secrets.py [--serial emulator-5554] [--apk PATH]

Clears the gallery's data (its files; the Keystore key goes with the app's data), runs phase 1
(the self-test: set, get, overwrite, delete, names, another app id, values up to SECRET_MAX; then
the key and the files from outside, one file damaged), checks that the shell user -- any other
uid -- cannot even list the app's no_backup folder while the app's own uid (run-as) sees the
encrypted files, stops the app, runs phase 2 (a new process: the values survived, the damaged
one is refused, all are deleted).  The APK must be a debug build.  Exit code 0 when everything
passes.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

PKG = "org.jflib.gallery"
RUNNER = f"{PKG}/.GallerySecretsInstrumentation"
SECRETS = f"/data/data/{PKG}/no_backup/jfgui-secrets"


def adb_path():
    for root in (os.environ.get("ANDROID_SDK_ROOT"), os.environ.get("ANDROID_HOME"),
                 os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk"),
                 os.path.join(Path.home(), "Android", "Sdk")):
        if root:
            p = Path(root) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb")
            if p.is_file():
                return str(p)
    return shutil.which("adb") or "adb"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--apk", help="install this APK first")
    a = ap.parse_args()
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])

    def run(*args, check=True, timeout=120):
        try:
            r = subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        except subprocess.TimeoutExpired:
            if check:
                sys.exit(f"adb {' '.join(args)}: no answer in {timeout} s")
            return f"timed out after {timeout} s"
        if check and r.returncode != 0:
            sys.exit(f"adb {' '.join(args)}: {r.stdout}{r.stderr}")
        return r.stdout + r.stderr

    if a.apk:
        print(run("install", "-r", a.apk).strip().splitlines()[-1])
    run("shell", "pm", "clear", PKG)
    failures = 0

    def phase(n):
        nonlocal failures
        out = run("shell", "am", "instrument", "-w", "-r", "-e", "phase", n, RUNNER, check=False, timeout=300)
        ok = "INSTRUMENTATION_CODE: -1" in out and "stream=PASS" in out
        stream = out[out.find("stream="):].split("INSTRUMENTATION_CODE")[0] if "stream=" in out else out
        print(f"phase {n}: {'PASS' if ok else 'FAIL'}\n{stream.strip()}\n", flush=True)
        failures += not ok
        run("shell", "am", "force-stop", PKG)

    phase("1")
    # another uid (the shell) does not get into the app's private storage; the app's own does
    other = run("shell", f"ls {SECRETS}", check=False).strip()
    own = run("shell", "run-as", PKG, "ls", "no_backup/jfgui-secrets", check=False).split()
    denied = "Permission denied" in other
    print(f"shell user: {other or '(nothing)'}")
    print(f"run-as {PKG}: {len(own)} files")
    if not denied or len(own) != 3:
        print("FAIL: another uid must not list the folder, the app's own must see three files")
        failures += 1
    phase("2")
    left = run("shell", "run-as", PKG, "ls", "no_backup/jfgui-secrets", check=False).split()
    print("no_backup/jfgui-secrets after phase 2:", " ".join(left) or "(empty)")
    print("android_secrets:", "ok" if failures == 0 else f"{failures} part(s) failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
