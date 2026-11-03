"""portabletest on an Android device: every lib/simd operation of the device's CPU against the scalar code.

    python apps/portabletest/tests/android.py [--serial SERIAL] [--exe PATH] [--digest HEX]

Pushes the executable (`python build_rs.py app portabletest --target android --android-abi ABI` makes
build/android/portabletest-<abi>; without --exe the one of the device's ABI) to /data/local/tmp, runs it
from `adb shell`, prints its output and the device's CPU, removes it again.  Nothing else on the device is
touched.  NEON is only checked on arm64-v8a: an x86_64 emulator runs SSE2.  --digest: the digest of another
target (win, linux, web) that this run must reproduce.

No device: SKIP, exit code 0.  Exit code 1 when a check failed.  adb comes from ANDROID_SDK_ROOT /
ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
REMOTE = "/data/local/tmp/jfgui_portabletest"


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
    ap.add_argument("--exe", help="the executable (default: build/android/portabletest-<the device's ABI>)")
    ap.add_argument("--digest", help="the digest another target printed")
    a = ap.parse_args()
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])

    def run(*args, check=True, timeout=300):
        r = subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        if check and r.returncode:
            raise SystemExit(f"FAIL adb {' '.join(args)}: {r.stdout}{r.stderr}")
        return r

    if run("get-state", check=False).stdout.strip() != "device":
        print("SKIP: no device")
        return 0
    abi = run("shell", "getprop", "ro.product.cpu.abi").stdout.strip()
    exe = Path(a.exe) if a.exe else ROOT / "build" / "android" / f"portabletest-{abi}"
    if not exe.is_file():
        print(f"SKIP: no {exe} (python build_rs.py app portabletest --target android --android-abi ABI)")
        return 0
    run("push", str(exe), REMOTE)
    try:
        run("shell", "chmod", "755", REMOTE)
        # (cd: the program may write beside it; the exit code through the output, older adb loses it)
        r = run("shell", f"cd /data/local/tmp && {REMOTE}; echo exit=$?", check=False)
    finally:
        run("shell", "rm", "-f", REMOTE, check=False)
    print(f"device: {abi}, {run('shell', 'getprop', 'ro.product.model').stdout.strip()}"
          f"{'' if abi == 'arm64-v8a' else ' (no NEON on this one)'}")
    sys.stdout.write(r.stdout)
    sys.stdout.write(r.stderr)
    m = re.search(r"^portabletest ok digest=([0-9a-f]{16})\s*$", r.stdout, re.M)
    if not m or not re.search(r"^exit=0\s*$", r.stdout, re.M):
        print("FAIL: portabletest")
        return 1
    if a.digest and m.group(1) != a.digest.lower():
        print(f"FAIL: digest {m.group(1)}, the other target printed {a.digest}")
        return 1
    print("PASS" + (": the same digest as the other target" if a.digest else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
