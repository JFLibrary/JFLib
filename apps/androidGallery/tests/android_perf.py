"""Android: what a frame of the gallery costs on a device while a page rolls.

    python apps/androidGallery/tests/android_perf.py [--serial SERIAL] [--apk PATH] [--page NAME] [--gpu 0|1]
                                                     [--swipes 12] [--rounds 1]

Starts the gallery with the extra jfgui_PERF, moves the page up and down with `input swipe` (500 ms each)
and prints what the app logged (`nativeRender` over each 120 changed frames, tag jfgui-perf) and
`dumpsys gfxinfo`, one line per round.  --gpu 0 (the default): the Bitmap's path, so the rasterizer's
time; nothing of that is in `nativeRender` on the GPU's path.  The numbers of different builds only
compare on the same page, with the same battery and temperature (printed), best in turns: --apk installs
the build first.  Android Studio's screen mirroring runs as a display of its own and skews the frame
times: a warning says so.  Changes nothing on the device but the app's own state; the app is stopped at
the end.  `input swipe` goes to whatever is in front and the phone is its owner's: this only starts from
the launcher or the gallery itself, and stops as soon as something else comes to the front (exit code 1).
The phone must be unlocked, the APK a debug build.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME /
%LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import re
import shutil
import statistics
import subprocess
import time
from pathlib import Path

PKG = "org.jflib.gallery"
ACTIVITY = f"{PKG}/.GalleryActivity"


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
    ap.add_argument("--page", help="the page to start on (jfgui_PAGE, default: the start page)")
    ap.add_argument("--gpu", default="0", help="jfgui_GPU (default 0: the Bitmap's path)")
    ap.add_argument("--swipes", type=int, default=12)
    ap.add_argument("--rounds", type=int, default=1)
    a = ap.parse_args()
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])

    def run(*args, timeout=120):
        return subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace",
                              timeout=timeout).stdout

    def sh(*args, timeout=120):
        return run("shell", *args, timeout=timeout)

    def in_front():
        # the focused window of each display: package/activity (the name of the package counts: other
        # apps of the library share the gallery's activity class)
        return re.findall(r"mCurrentFocus=Window\{\S+ \S+ ([^/}\s]+)/", sh("dumpsys window | grep mCurrentFocus"))

    def stop_unless(allowed, when):
        front = in_front()
        if not any(p == PKG or (allowed == "launcher" and "launcher" in p) for p in front):
            raise SystemExit(f"STOP {when}: {', '.join(front) or 'nothing'} in front, the phone is in use")

    stop_unless("launcher", "before the start")
    if a.apk:
        print(run("install", "-r", a.apk, timeout=300).strip().splitlines()[-1], flush=True)
    if "studio.screen.sharing" in sh("dumpsys", "display"):
        print("WARNING: Android Studio mirrors the screen: frame times are skewed", flush=True)
    w, h = (int(v) for v in re.search(r"(\d+)x(\d+)", sh("wm", "size")).groups())
    extras = ["--es", "jfgui_PERF", "1", "--es", "jfgui_LANG", "de", "--es", "jfgui_GPU", a.gpu]
    if a.page:
        extras += ["--es", "jfgui_PAGE", a.page]
    sh("input", "keyevent", "KEYCODE_WAKEUP")
    for k in range(a.rounds):
        stop_unless("launcher", f"before round {k + 1}")
        sh("am", "force-stop", PKG)
        sh("am", "start", "-W", "-n", ACTIVITY, *extras, timeout=60)
        time.sleep(3)
        run("logcat", "-c")
        sh("dumpsys", "gfxinfo", PKG, "reset")
        for i in range(a.swipes):
            stop_unless("gallery", f"before swipe {i + 1}")
            y0, y1 = (h * 3 // 4, h // 4) if i % 2 == 0 else (h // 4, h * 3 // 4)
            sh("input", "swipe", str(w // 2), str(y0), str(w // 2), str(y1), "500")
        time.sleep(1)
        info = sh("dumpsys", "gfxinfo", PKG)
        logged = re.findall(r"nativeRender over \d+ changed frames: mean ([\d.]+) ms, median ([\d.]+), p95 ([\d.]+), max ([\d.]+) "
                            r"\((\d+x\d+)\), of the mean presenting ([\d.]+)", run("logcat", "-d", "-s", "jfgui-perf"))

        def gfx(pattern):
            m = re.search(pattern, info)
            return m.group(1) if m else "?"

        means, medians, p95s, presents = ([float(row[i]) for row in logged] for i in (0, 1, 2, 5))
        render = (f"nativeRender {len(logged) * 120} frames {logged[0][4]}: mean {statistics.mean(means):.2f}, "
                  f"median {statistics.median(medians):.2f}, p95 {statistics.median(p95s):.2f}, presenting "
                  f"{statistics.mean(presents):.2f} ms") if logged else "nativeRender: nothing logged"
        temp = re.search(r"temperature: (\d+)", sh("dumpsys", "battery"))
        print(f"round {k + 1}: {render}; gfxinfo {gfx(r'Total frames rendered: (\d+)')} frames, janky "
              f"{gfx(r'Janky frames: \d+ \(([\d.]+%)\)')}, p50 {gfx(r'50th percentile: (\d+ms)')}, p95 "
              f"{gfx(r'95th percentile: (\d+ms)')}, missed vsyncs {gfx(r'Number Missed Vsync: (\d+)')}; "
              f"battery {int(temp.group(1)) / 10 if temp else '?'} C", flush=True)
    sh("am", "force-stop", PKG)


if __name__ == "__main__":
    main()
