"""gputest on an Android device or emulator: the checks of lib/gpu that need no window.

    python apps/gputest/tests/android.py [--serial SERIAL] [--exe PATH] [--test NAME] [--info]

Pushes the executable (`python build_rs.py app gputest --target android --android-abi ABI` makes
build/android/gputest-<abi>; without --exe the one of the device's ABI) to /data/local/tmp, runs
`gputest --test all` (or --test NAME) from `adb shell`, prints its output, removes it again.  A
program from `adb shell` has no app and so no surface: the tests that want a window SKIP, the
swapchain is tested by apps/androidGallery/tests/android_gpu.py.  Nothing else on the device is touched.

No device: SKIP, exit code 0.  A device without a usable GPU (the emulator's -gpu swiftshader_indirect
has Vulkan 1.2 without dynamic rendering, needs -gpu host): gputest says SKIP itself, exit code 0.
Exit code 1 when a check failed.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
REMOTE = "/data/local/tmp/jfgui_gputest"


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
    ap.add_argument("--exe", help="the executable (default: build/android/gputest-<the device's ABI>)")
    ap.add_argument("--test", default="all")
    ap.add_argument("--info", action="store_true")
    a = ap.parse_args()
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])

    def run(*args, check=True, timeout=120):
        r = subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        if check and r.returncode:
            raise SystemExit(f"FAIL adb {' '.join(args)}: {r.stdout}{r.stderr}")
        return r

    if run("get-state", check=False).stdout.strip() != "device":
        print("SKIP: no device")
        return 0
    exe = Path(a.exe) if a.exe else ROOT / "build" / "android" / f"gputest-{run('shell', 'getprop', 'ro.product.cpu.abi').stdout.strip()}"
    if not exe.is_file():
        print(f"SKIP: no {exe} (python build_rs.py app gputest --target android --android-abi ABI)")
        return 0
    run("push", str(exe), REMOTE)
    try:
        run("shell", "chmod", "755", REMOTE)
        what = ["--info"] if a.info else ["--test", a.test]
        r = run("shell", REMOTE, *what, check=False, timeout=900)
        sys.stdout.write(r.stdout + r.stderr)
        return 1 if r.returncode else 0
    finally:
        run("shell", "rm", "-f", REMOTE, check=False)


if __name__ == "__main__":
    os.environ.setdefault("MSYS_NO_PATHCONV", "1")
    sys.exit(main())
