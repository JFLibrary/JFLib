"""Android: sound output (plat::audio over AAudio: android/audio.c, lib/plat/android_audio.rs)
on a device or an emulator with an audio device.

    python apps/audiotest/tests/android_audio.py [--serial emulator-5570] [--apk PATH]

Installs the APK, clears the app's data and starts it with jfgui_SELFTEST=audio
(apps/audiotest/src/selftest.rs: a quiet fill stream that plays, pauses, plays 4 s more and
stops; a play() voice; the system's information sound).  While the stream plays, the audio
server must report it: dumpsys audio lists an AAudio player of the app's uid in state
"started", and dumpsys media.audio_flinger an output track of that uid.  Then the self-test's
results (run-as: files/fs/data/audiotest/audio-selftest.txt) must all be "ok".  The APK must be
a debug build.  Exit code 0 when everything passes.  adb comes from ANDROID_SDK_ROOT /
ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

PKG = "org.jflib.audiotest"
ACTIVITY = f"{PKG}/org.jflib.gallery.GalleryActivity"
RESULTS = "files/fs/data/audiotest/audio-selftest.txt"


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
    m = re.search(r"uid:(\d+)", run("shell", "pm", "list", "packages", "-U", PKG))
    if not m:
        sys.exit(f"{PKG} is not installed")
    uid = m.group(1)
    print(f"{PKG}: uid {uid}")
    run("shell", "am", "start", "-W", "-n", ACTIVITY, "--es", "jfgui_SELFTEST", "audio")
    failures = 0
    # the stream plays from about 0.5 s to 5 s after the start (with a pause after 0.8 s)
    player = track = False
    end = time.time() + 6
    while time.time() < end and not (player and track):
        audio = run("shell", "dumpsys", "audio", check=False)
        flinger = run("shell", "dumpsys", "media.audio_flinger", check=False)
        player = player or any(f"u/pid:{uid}/" in ln and "state:started" in ln for ln in audio.splitlines())
        # the app's pids (its notification client, its sessions), then an active track of one:
        # "Type Id Active Client Session ..." rows read "<id> yes <pid> <session> ..."
        pids = {m.group(1) for m in re.finditer(rf"^\s+(\d+)\s+{uid}\s+{re.escape(PKG)}\s*$", flinger, re.M)}
        pids |= {m.group(1) for m in re.finditer(rf"^\s+\d+\s+\d+\s+(\d+)\s+{uid}\s+{re.escape(PKG)}\s*$", flinger, re.M)}
        track = track or any(m.group(1) in pids for m in re.finditer(r"^\s+\d+\s+yes\s+(\d+)\s+\d+\s", flinger, re.M))
        if not (player and track):
            time.sleep(0.4)
    for ok, what in ((player, f"dumpsys audio: a started player of uid {uid}"),
                     (track, f"dumpsys media.audio_flinger: an output track of uid {uid}")):
        print(f"{'PASS' if ok else 'FAIL'} {what}")
        failures += not ok
    text = ""
    end = time.time() + 20
    while time.time() < end and "done " not in text:
        text = run("shell", "run-as", PKG, "cat", RESULTS, check=False)
        if "done " not in text:
            time.sleep(0.5)
    print(text.strip() or "(no results)")
    lines = [ln for ln in text.splitlines() if ln.strip()]
    if not lines or not lines[-1].startswith("done ") or any(ln.startswith("FAIL") for ln in lines):
        print("FAIL the self-test")
        failures += 1
    run("shell", "am", "force-stop", PKG)
    print("android_audio:", "ok" if failures == 0 else f"{failures} part(s) failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
