"""JFLib's Android tests on an emulator of their own (docs/ANDROID.md, AN5; .github/workflows/android.yml).

    python tools/android_ci.py [--port 5554] [--no-build] [--keep] [--only NAME ...] [--dry-run]

Builds the programs without a UI for x86_64 (build_rs.py app ... --target android: coretest, plattest,
asynctest, jfWord with jfgui_ANDROID_CONSOLE=1), makes an AVD in a folder of its own from the system
image (x86-64, API 35, no Google APIs), starts the emulator without a window, waits for the boot and
runs each test per `adb shell` and in its test APK (tools/android_console.py: the app's process with
Zygote's seccomp filter).  The emulator is stopped and the AVD removed at the end (--keep leaves both).

Nothing is downloaded: the SDK (ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk) must
hold the emulator, the image, platform-tools, cmdline-tools, build-tools, platforms;android-36 and the
NDK (ANDROID_NDK_HOME); the workflow installs them with sdkmanager.  On Linux the emulator needs
/dev/kvm (read and write; the workflow opens it with a udev rule).  Exit code 1 when a test failed or
the emulator did not boot; --dry-run prints the steps without running them.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IMAGE = "system-images;android-35;default;x86_64"
ABI = "x86_64"
WINDOWS = os.name == "nt"

# (name, build_rs.py app NAME, extra environment of the build)
BUILDS = [("coretest", {}), ("plattest", {}), ("asynctest", {}), ("jfWord", {"jfgui_ANDROID_CONSOLE": "1"})]
# (name, the command after `python`; SERIAL is replaced by the emulator's)
TESTS = [
    ("coretest", ["tools/android_console.py", "coretest", "--quiet", "--serial", "SERIAL"]),
    ("plattest all", ["tools/android_console.py", "plattest", "--quiet", "--serial", "SERIAL", "--", "--test", "all"]),
    ("plattest threads", ["tools/android_console.py", "plattest", "--quiet", "--serial", "SERIAL", "--", "--test", "threads"]),
    ("plattest kernel", ["tools/android_console.py", "plattest", "--quiet", "--serial", "SERIAL", "--", "--test", "kernel"]),
    ("plattest vfs", ["tools/android_console.py", "plattest", "--quiet", "--serial", "SERIAL", "--", "--test", "vfs"]),
    ("asynctest tasks.uis", ["tools/android_console.py", "asynctest", "--quiet", "--serial", "SERIAL",
                             "--file", "apps/asynctest/tests/tasks.uis", "--", "--script", "tasks.uis"]),
    ("asynctest copy.uis", ["tools/android_console.py", "asynctest", "--quiet", "--serial", "SERIAL",
                            "--file", "apps/asynctest/tests/copy.uis", "--dir", "build", "--", "--script", "copy.uis"]),
    ("jfWord", ["apps/jfWord/tests/android.py", "--quiet", "--serial", "SERIAL"]),
]


def sdk_root():
    for p in (os.environ.get("ANDROID_SDK_ROOT"), os.environ.get("ANDROID_HOME"),
              os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk") if WINDOWS else None,
              str(Path.home() / "Android" / "Sdk")):
        if p and (Path(p) / "platform-tools").is_dir():
            return Path(p)
    raise SystemExit("FAIL: no Android SDK (ANDROID_SDK_ROOT or ANDROID_HOME)")


def tool(sdk, *parts):
    """an SDK program by its path below the SDK, with Windows' suffix (.exe, .bat for cmdline-tools)"""
    p = sdk.joinpath(*parts)
    if WINDOWS:
        for suffix in (".exe", ".bat"):
            if p.with_name(p.name + suffix).is_file():
                return p.with_name(p.name + suffix)
    return p


def check_sdk(sdk, dry):
    missing = [str(p) for p in (tool(sdk, "emulator", "emulator"), tool(sdk, "platform-tools", "adb"),
                                tool(sdk, "cmdline-tools", "latest", "bin", "avdmanager"),
                                sdk.joinpath(*IMAGE.split(";")))
               if not Path(p).exists()]
    if missing and not dry:
        raise SystemExit("FAIL: missing in the SDK (sdkmanager installs them): " + ", ".join(missing))
    if sys.platform.startswith("linux"):
        kvm = Path("/dev/kvm")
        if not kvm.exists() or not os.access(kvm, os.R_OK | os.W_OK):
            message = ("/dev/kvm is missing or not readable and writable: the emulator needs KVM "
                       "(the workflow's udev rule, or membership of the group kvm)")
            if not dry:
                raise SystemExit("FAIL: " + message)
            print("note: " + message)


def run(command, dry, env=None, **kw):
    print("+ " + " ".join(str(c) for c in command), flush=True)
    if dry:
        return subprocess.CompletedProcess(command, 0, "", "")
    return subprocess.run([str(c) for c in command], env=env, **kw)


def adb_out(adb, serial, *args, timeout=30):
    try:
        r = subprocess.run([str(adb), "-s", serial, *args], capture_output=True, text=True, timeout=timeout,
                           stdin=subprocess.DEVNULL)
        return r.stdout.strip()
    except subprocess.TimeoutExpired:
        return ""


def wait_boot(adb, serial, emulator, timeout):
    """-> seconds until sys.boot_completed is 1, or None (the emulator ended or the time ran out)"""
    t0 = time.monotonic()
    while time.monotonic() - t0 < timeout:
        if emulator.poll() is not None:
            print(f"FAIL: the emulator ended with exit code {emulator.returncode}")
            return None
        if adb_out(adb, serial, "shell", "getprop", "sys.boot_completed") == "1":
            return time.monotonic() - t0
        time.sleep(3)
    print(f"FAIL: no boot within {timeout} s")
    return None


def stop(adb, serial, emulator):
    if emulator.poll() is None:
        adb_out(adb, serial, "emu", "kill", timeout=20)
        try:
            emulator.wait(60)
        except subprocess.TimeoutExpired:
            emulator.kill()
            emulator.wait(30)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--port", type=int, default=5554, help="the emulator's console port (adb: emulator-PORT)")
    ap.add_argument("--no-build", action="store_true", help="take the builds in build/android as they are")
    ap.add_argument("--keep", action="store_true", help="leave the emulator running and the AVD in place")
    ap.add_argument("--only", action="append", default=[], help="only the tests whose name starts with this")
    ap.add_argument("--boot-timeout", type=int, default=600)
    ap.add_argument("--memory", type=int, default=3072, help="the emulator's RAM in MB")
    ap.add_argument("--log", default=str(ROOT / "build" / "android" / "emulator.log"))
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    dry = a.dry_run
    sdk = sdk_root() if not dry else Path(os.environ.get("ANDROID_SDK_ROOT") or os.environ.get("ANDROID_HOME") or "SDK")
    check_sdk(sdk, dry)
    tests = [t for t in TESTS if not a.only or any(t[0].startswith(o) for o in a.only)]
    if not tests:
        raise SystemExit("FAIL: --only matches no test: " + ", ".join(n for n, _ in TESTS))

    # 1. the builds
    if not a.no_build:
        for name, extra in BUILDS:
            if a.only and not any(t[0].split()[0] == name for t in tests):
                continue
            r = run([sys.executable, ROOT / "build_rs.py", "app", name, "--target", "android", "--android-abi", ABI], dry,
                    env={**os.environ, **extra}, cwd=ROOT)
            if r.returncode:
                print(f"FAIL: build of {name}")
                return 1

    # 2. the AVD in a folder of its own, the emulator without a window
    adb = tool(sdk, "platform-tools", "adb")
    serial = f"emulator-{a.port}"
    home = Path(tempfile.mkdtemp(prefix="jfgui-avd-")) if not dry else Path("AVD_HOME")
    env = {**os.environ, "ANDROID_AVD_HOME": str(home), "ANDROID_SDK_ROOT": str(sdk),
           # the SDK's adb first: the emulator talks to the adb server of the PATH
           "PATH": str(sdk / "platform-tools") + os.pathsep + os.environ.get("PATH", "")}
    r = run([tool(sdk, "cmdline-tools", "latest", "bin", "avdmanager"), "create", "avd", "-n", "jfgui_ci", "-k", IMAGE,
             "-d", "pixel_6", "-p", home / "jfgui_ci.avd"], dry, env=env, input="no\n", text=True, capture_output=not dry)
    if r.returncode:
        print(f"FAIL: avdmanager create avd\n{r.stdout}{r.stderr}")
        return 1
    command = [tool(sdk, "emulator", "emulator"), "-avd", "jfgui_ci", "-port", str(a.port), "-no-window", "-no-audio",
               "-no-boot-anim", "-no-snapshot", "-gpu", "swiftshader_indirect", "-memory", str(a.memory)]
    print("+ " + " ".join(str(c) for c in command), flush=True)
    if dry:
        for name, test in tests:
            print("+ " + " ".join([Path(sys.executable).name] + [serial if w == "SERIAL" else w for w in test]))
        print("+ adb -s " + serial + " emu kill")
        return 0
    Path(a.log).parent.mkdir(parents=True, exist_ok=True)
    log = open(a.log, "wb")
    emulator = subprocess.Popen([str(c) for c in command], env=env, stdout=log, stderr=subprocess.STDOUT,
                                stdin=subprocess.DEVNULL)
    results = []
    try:
        took = wait_boot(adb, serial, emulator, a.boot_timeout)
        if took is None:
            return 1
        print(f"booted in {took:.0f} s: Android {adb_out(adb, serial, 'shell', 'getprop', 'ro.build.version.release')}, "
              f"{adb_out(adb, serial, 'shell', 'getprop', 'ro.product.cpu.abi')}", flush=True)
        # 3. the tests, one after the other
        for name, test in tests:
            t0 = time.monotonic()
            r = subprocess.run([sys.executable, *[serial if w == "SERIAL" else w for w in test]], cwd=ROOT, env=env,
                               capture_output=True, text=True, encoding="utf-8", errors="replace",
                               stdin=subprocess.DEVNULL)
            out = (r.stdout + r.stderr).strip()
            # a SKIP (no device, no build) is a failure here: the emulator and the builds are ours
            last = out.splitlines()[-1] if out else ""
            ok = r.returncode == 0 and last.startswith("PASS")
            results.append((name, ok, time.monotonic() - t0))
            print(f"::group::{name}" if os.environ.get("GITHUB_ACTIONS") else f"=== {name}")
            print(out)
            if os.environ.get("GITHUB_ACTIONS"):
                print("::endgroup::")
            print(f"{'PASS' if ok else 'FAIL'} {name} ({results[-1][2]:.0f} s)", flush=True)
    finally:
        if not a.keep:
            stop(adb, serial, emulator)
            shutil.rmtree(home, ignore_errors=True)
        else:
            print(f"kept: {serial}, ANDROID_AVD_HOME={home}")
        log.close()
    failed = [n for n, ok, _ in results if not ok]
    print(f"{len(results) - len(failed)} of {len(results)} passed" + (": FAIL " + ", ".join(failed) if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
