"""Android: the monitor list (lib/plat/web_screen.rs jfgui_screens, android/GalleryActivity.java
screens(): the DisplayManager's displays, android/bridge.c nativeScreens) on a device or an
emulator.

    python apps/screentest/tests/android_screen.py [--serial emulator-5560] [--apk PATH]

Installs the APK, clears the app's data and starts it with jfgui_SELFTEST=screen: the app writes
its monitors (ui.monitors) to data://screen-selftest.txt at the start and after every change
(apps/screentest/src/main.rs).  The default display must match what the system says on its own
(wm size, wm density, dumpsys display: size, 96 * density / 160 dpi, refresh rate, primary,
name); then `wm size` changes the display: the app must hear it (UIC_SETTINGS with
SET_MONITORS, the new size) and again after `wm size reset`.  The APK must be a debug build.
Exit code 0 when everything passes.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME /
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

PKG = "org.jflib.screentest"
ACTIVITY = f"{PKG}/org.jflib.gallery.GalleryActivity"
RESULTS = "files/fs/data/screentest/screen-selftest.txt"


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
    failures = 0

    def run(*args, check=True, timeout=120):
        r = subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        if check and r.returncode != 0:
            sys.exit(f"adb {' '.join(args)}: {r.stdout}{r.stderr}")
        return r.stdout + r.stderr

    def check(ok, what, info=""):
        nonlocal failures
        print(("PASS " if ok else "FAIL ") + what + ("" if ok else f": {info}"))
        failures += 0 if ok else 1

    def results(want_changes, timeout=20):
        """the self-test's monitors once it reports at least want_changes changes"""
        end = time.time() + timeout
        text = ""
        while time.time() < end:
            text = run("shell", "run-as", PKG, "cat", RESULTS, check=False)
            m = re.search(r"^changes (\d+)", text, re.M)
            if m and int(m.group(1)) >= want_changes:
                break
            time.sleep(0.5)
        mons = []
        for ln in text.splitlines():
            if ln.startswith("monitor "):
                f = ln.split(" ", 13)
                mons.append({"id": int(f[1]), "rect": [int(x) for x in f[2:6]], "work": [int(x) for x in f[6:10]], "dpi": int(f[10]),
                             "mhz": int(f[11]), "primary": f[12] == "1", "name": f[13] if len(f) > 13 else ""})
        m = re.search(r"^changes (\d+)", text, re.M)
        return mons, int(m.group(1)) if m else -1, text

    if a.apk:
        print(run("install", "-r", a.apk).strip().splitlines()[-1])
    run("shell", "wm", "size", "reset", check=False)
    run("shell", "pm", "clear", PKG)
    run("shell", "am", "start", "-W", "-n", ACTIVITY, "--es", "jfgui_SELFTEST", "screen")
    mons, changes, text = results(0)
    print(text.strip() or "(no results)")
    # what the system says on its own
    size = re.search(r"Physical size: (\d+)x(\d+)", run("shell", "wm", "size"))
    density = re.search(r"Physical density: (\d+)", run("shell", "wm", "density"))
    disp = run("shell", "dumpsys", "display")
    fps = re.search(r"renderFrameRate ([\d.]+)", disp) or re.search(r"fps=([\d.]+)", disp)
    name = re.search(r'DisplayDeviceInfo\{"([^"]+)"', disp)
    w, h = (int(size.group(1)), int(size.group(2))) if size else (0, 0)
    dpi = round(96 * int(density.group(1)) / 160) if density else 0
    mhz = round(float(fps.group(1)) * 1000) if fps else 0
    print(f"system: {w}x{h}, density {density.group(1) if density else '?'} -> {dpi} dpi, {mhz} mHz, {name.group(1) if name else '?'}")
    first = mons[0] if mons else None
    check(first is not None and first["primary"], "the default display is listed first, primary", text)
    if first:
        check(sorted(first["rect"][2:]) == sorted([w, h]) and first["rect"][:2] == [0, 0], f"its size is wm size's ({w} x {h})", first)
        check(first["work"] == first["rect"], "the work area is the display", first)
        check(abs(first["dpi"] - dpi) <= 1, f"dpi 96 * density / 160 = {dpi}", first)
        check(abs(first["mhz"] - mhz) <= 1000, f"refresh {mhz} mHz", first)
        check(not name or first["name"] == name.group(1), "the display's name", first)
    check(changes == 0, "no change at the start", changes)
    # a change of the display: wm size (the app hears it), then back
    nw, nh = (w * 3 // 4) // 2 * 2, (h * 3 // 4) // 2 * 2
    run("shell", "wm", "size", f"{nw}x{nh}")
    mons2, changes2, text2 = results(1)
    check(changes2 >= 1 and mons2 and sorted(mons2[0]["rect"][2:]) == sorted([nw, nh]), f"wm size {nw}x{nh}: UIC_SETTINGS with SET_MONITORS and the new size", text2)
    run("shell", "wm", "size", "reset")
    mons3, changes3, text3 = results(changes2 + 1)
    check(changes3 > changes2 and mons3 and sorted(mons3[0]["rect"][2:]) == sorted([w, h]), "wm size reset: the old size again", text3)
    run("shell", "am", "force-stop", PKG)
    print("android_screen:", "ok" if failures == 0 else f"{failures} check(s) failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
