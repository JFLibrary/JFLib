"""Android: the gallery's UI stays smooth while a long job runs on the pool's threads, and a job that ends
while the UI rests wakes it at once (docs/ANDROID.md, AN1; apps/androidGallery/jobtest.rs,
lib/plat/android_thread.rs, android/bridge.c).

    python apps/androidGallery/tests/android_job.py [--serial SERIAL] [--apk PATH] [--gpu 0|1] [--extra NAME=VALUE ...]
                                                     [--max-gap-ms 150] [--max-over 2] [--min-rate 0.5]
                                                     [--max-wake-ms 100] [--rounds 1]

Starts the gallery with jfgui_SELFTEST=job: after 1.5 s to come up, one second of frames that a 16 ms
timer asks for (an animation) without a job, the device's own pace; then 240 items of 20 ms arithmetic on
plat::Job with the same frames (card 01's progress bar follows the job); then five jobs of one sleeping
item while the UI asks for nothing.  The app writes one line to data://job-state.txt (read with run-as)
and to logcat: the pool's workers, the frames and the longest gap without the job, how long the job took,
the frames meanwhile, the longest gap and the gaps over 50 ms (ms@ms after the job's start), the time
from each sleeping item's end to the UI's wake-up (UIC_WAKE).  Passes when the pool has workers (real
threads, not items between frames), at most --max-over gaps during the job are longer than --max-gap-ms
(an emulator on a busy computer misses a frame now and then), the frame rate during the job is at least
--min-rate of the one without it, and no wake-up takes longer than --max-wake-ms (an idle host ticks
every 250 ms: without the looper's wake-up these would be up to 250 ms).  Items between frames on the UI
thread, as before AN1, would stop it for 16 x 20 ms every tick.  After an install one start is thrown
away first (the first start compiles).
Also prints `dumpsys gfxinfo`'s janky frames of the run (frames through the GPU's SurfaceView are not
among them).

Without --apk it installs build/android/androidGallery-<ro.product.cpu.abi>-debug.apk when that exists;
the APK must be a debug build (jfgui_* extras and run-as).  The phone is its owner's: this starts only
from the launcher or the gallery itself (exit code 1 otherwise); it injects no input.  The app is stopped
at the end.  Exit code 0 when every round passes.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME /
%LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

PKG = "org.jflib.gallery"
ACTIVITY = f"{PKG}/.GalleryActivity"
STATE = "files/fs/data/androidGallery/job-state.txt"
ROOT = Path(__file__).resolve().parents[3]


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
    ap.add_argument("--apk", help="install this APK first (default: build/android/androidGallery-<abi>-debug.apk if it exists)")
    ap.add_argument("--gpu", help="jfgui_GPU for the run (default: the app's own choice)")
    ap.add_argument("--extra", action="append", default=[], help="another jfgui_* extra of the start (jfgui_RENDER_PAR=0)")
    ap.add_argument("--max-gap-ms", type=int, default=150)
    ap.add_argument("--max-over", type=int, default=2, help="gaps longer than --max-gap-ms allowed during the job")
    ap.add_argument("--min-rate", type=float, default=0.5, help="frames per second during the job / without it")
    ap.add_argument("--max-wake-ms", type=int, default=100)
    ap.add_argument("--rounds", type=int, default=1)
    a = ap.parse_args()
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])

    def run(*args, timeout=120):
        r = subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=timeout)
        return r.stdout

    def sh(*args, timeout=120):
        return run("shell", *args, timeout=timeout)

    def in_front():
        return re.findall(r"mCurrentFocus=Window\{\S+ \S+ ([^/}\s]+)/", sh("dumpsys window | grep mCurrentFocus"))

    def stop_unless_free(when):
        front = in_front()
        if not any(p == PKG or "launcher" in p for p in front):
            raise SystemExit(f"STOP {when}: {', '.join(front) or 'nothing'} in front, the phone is in use")

    abi = sh("getprop", "ro.product.cpu.abi").strip()
    print(f"device: {sh('getprop', 'ro.product.model').strip()} (API {sh('getprop', 'ro.build.version.sdk').strip()}, {abi})",
          flush=True)
    stop_unless_free("before the start")
    apk = Path(a.apk) if a.apk else ROOT / "build" / "android" / f"androidGallery-{abi}-debug.apk"
    if a.apk or apk.is_file():
        if not apk.is_file():
            sys.exit(f"no APK {apk} (python build_rs.py app androidGallery --target android --android-abi {abi})")
        print(run("install", "-r", str(apk), timeout=300).strip().splitlines()[-1], flush=True)
        sh("am", "start", "-W", "-n", ACTIVITY)  # the first start after an install compiles: not measured
        time.sleep(4)

    failures = 0
    try:
        for k in range(a.rounds):
            sh("am", "force-stop", PKG)
            sh("run-as", PKG, "rm", "-f", STATE)
            sh("dumpsys", "gfxinfo", PKG, "reset")
            extras = ["--es", "jfgui_SELFTEST", "job", "--es", "jfgui_LANG", "de"]
            if a.gpu is not None:
                extras += ["--es", "jfgui_GPU", a.gpu]
            for e in a.extra:
                name, _, value = e.partition("=")
                extras += ["--es", name, value]
            sh("am", "start", "-W", "-n", ACTIVITY, *extras)
            line = ""
            deadline = time.time() + 120
            while time.time() < deadline:
                line = sh("run-as", PKG, "cat", STATE).strip()
                if line.startswith("jobtest:"):
                    break
                if PKG not in in_front():
                    print("FAIL the gallery left the front before its result", flush=True)
                    break
                time.sleep(1)
            gfx = sh("dumpsys", "gfxinfo", PKG)
            janky = re.search(r"Janky frames: (\d+) \(([\d.]+)%\)", gfx)
            total = re.search(r"Total frames rendered: (\d+)", gfx)
            print(f"round {k + 1}: {line or '(no result)'}", flush=True)
            print(f"  gfxinfo: {total.group(1) if total else '?'} frames, janky {janky.group(1) + ' (' + janky.group(2) + ' %)' if janky else '?'}",
                  flush=True)
            v = dict(kv.split("=", 1) for kv in line.split()[1:] if "=" in kv)
            wakes = [int(w) for w in v.get("wake_ms", "").split(",") if w]
            base_rate = int(v.get("base_frames", "0"))  # over one second
            job_rate = int(v.get("frames", "0")) * 1000 / max(1, int(v.get("job_ms", "1")))
            over = [int(g.split("@")[0]) for g in v.get("slow", "").split(",") if "@" in g]
            over = [g for g in over if g > a.max_gap_ms]
            checks = [
                (int(v.get("workers", "0")) > 0, "the pool has worker threads"),
                (int(v.get("frames", "0")) > 0 and len(over) <= a.max_over and int(v.get("slow_frames", "0")) <= 64,
                 f"frames kept coming during the job ({len(over)} gaps over {a.max_gap_ms} ms, at most {a.max_over}; "
                 f"longest {v.get('max_gap_ms', '?')} ms, without the job {v.get('base_gap_ms', '?')} ms)"),
                (base_rate > 0 and job_rate >= a.min_rate * base_rate,
                 f"the frame rate held during the job ({job_rate:.0f}/s, without it {base_rate}/s, at least {a.min_rate:.0%})"),
                (len(wakes) == 5 and max(wakes) <= a.max_wake_ms,
                 f"a job's end wakes the resting UI (longest {max(wakes) if wakes else '?'} ms <= {a.max_wake_ms})"),
            ]
            for good, what in checks:
                print(f"  {'ok  ' if good else 'FAIL'} {what}", flush=True)
                failures += not good
    finally:
        sh("am", "force-stop", PKG)
    print("PASS" if failures == 0 else f"FAIL ({failures})", flush=True)
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
