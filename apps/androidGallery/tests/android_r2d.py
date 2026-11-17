"""Android: the page "2D-Zeichnung" (pg_r2d.rs, G6e3) runs its check or its measurement on the device's GPU and logs it.

    python apps/androidGallery/tests/android_r2d.py [--serial SERIAL] [--apk PATH] [--package NAME] [--check] [--bench]
                                                    [--runs 3]

--check (the default without --bench): jfgui_GPU_R2DCHECK=1, render2dtest's 91 images on the GPU against the CPU path
on the same device (the cpu test's bounds), the CPU path's bytes against every other target's, the drawing at 1e7, lines,
rings and dash ends at 1e-6 units per pixel, two DXF drawings; the lines "r2dcheck: ..." of logcat (tag jfgui), and
"done: N checks, M failed" at the end.  --bench: jfgui_GPU_R2DBENCH=1, the town plan of 1 million records with and without
text at three views, --runs times (one start each, in turns with nothing); the lines "r2dbench: ...".  Each start ends
by itself (the activity finishes).

It installs the APK when given (a debug build: the phone's keeps the debug key the gallery was signed with), starts the
gallery only when the launcher or the gallery itself is in front (the phone is its owner's: it stops when something else
is), injects no input, changes no setting, and stops the app at the end.  The phone must be unlocked.  No device: SKIP.
adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.  Exit code 1 when a check failed.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path


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
    os.environ.setdefault("MSYS_NO_PATHCONV", "1")
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--apk", help="install this APK first")
    ap.add_argument("--package", default="org.jflib.gallery", help="the APK's package (a test build: jfgui_ANDROID_PACKAGE)")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--runs", type=int, default=3, help="--bench: starts, in turns")
    ap.add_argument("--timeout", type=int, default=600, help="seconds a start may take")
    a = ap.parse_args()
    if not a.bench:
        a.check = True
    pkg = a.package
    activity = f"{pkg}/org.jflib.gallery.GalleryActivity"
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])

    def run(*args, timeout=120):
        try:
            return subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout).stdout
        except subprocess.TimeoutExpired:
            return ""

    if run("get-state").strip() != "device":
        print("SKIP: no device")
        return 0

    def in_front():
        # the focused window of each display (package/activity); with none (mCurrentFocus=null, a fresh emulator) the
        # focused activity
        text = run("shell", "dumpsys window | grep -E 'mCurrentFocus|mFocusedApp'")
        return re.findall(r"mCurrentFocus=Window\{\S+ \S+ ([^/}\s]+)/", text) or re.findall(r"mFocusedApp=ActivityRecord\{\S+ \S+ ([^/}\s]+)/", text)

    def stop_unless(when):
        front = in_front()
        if not any(p == pkg or "launcher" in p for p in front):
            raise SystemExit(f"STOP {when}: {', '.join(front) or 'nothing'} in front, the phone is in use")

    stop_unless("before the start")
    model = run("shell", "getprop", "ro.product.model").strip()
    abi = run("shell", "getprop", "ro.product.cpu.abi").strip()
    gpu = re.search(r"GLES: ([^\n]+)", run("shell", "dumpsys", "SurfaceFlinger"))
    print(f"device: {model}, {abi}, Android {run('shell', 'getprop', 'ro.build.version.release').strip()}; {gpu.group(1).strip() if gpu else 'GLES ?'}")
    if a.apk:
        print(run("install", "-r", a.apk, timeout=300).strip().splitlines()[-1], flush=True)
    fails = 0

    def start(key, tag, timeout):
        stop_unless(f"before {tag}")
        run("shell", "am", "force-stop", pkg)
        run("logcat", "-c")
        run("shell", "am", "start", "-W", "-n", activity, "--es", "jfgui_LANG", "de", "--es", key, "1", timeout=60)
        lines, done, t0, shown = [], None, time.time(), 0
        while time.time() < t0 + timeout and done is None:
            time.sleep(2.0)
            log = run("logcat", "-d", "-s", "jfgui:I")
            lines = [ln.split(f"{tag}: ", 1)[1].strip() for ln in log.splitlines() if f"{tag}: " in ln]
            for ln in lines[shown:]:
                print(f"  {ln}", flush=True)
            shown = len(lines)
            done = next((ln for ln in lines if ln.startswith("done")), None)
        run("shell", "am", "force-stop", pkg)
        return lines, done

    if a.check:
        lines, done = start("jfgui_GPU_R2DCHECK", "r2dcheck", a.timeout)
        m = re.search(r"done: (\d+) checks, (\d+) failed", done or "")
        good = bool(m) and m.group(2) == "0" and int(m.group(1)) >= 30
        print(f"{'PASS' if good else 'FAIL'} the 2D drawing's check: {done or f'no end in {a.timeout} s'}")
        fails += not good
    if a.bench:
        for k in range(a.runs):
            print(f"run {k + 1} of {a.runs}:")
            lines, done = start("jfgui_GPU_R2DBENCH", "r2dbench", a.timeout)
            good = done == "done" and not any(ln.startswith("FAIL") for ln in lines)
            print(f"{'PASS' if good else 'FAIL'} the measurement, run {k + 1}: {done or f'no end in {a.timeout} s'}")
            fails += not good
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
