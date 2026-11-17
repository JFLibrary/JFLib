"""A program without a UI on an Android device or emulator: per `adb shell` and in an app's process.

    python tools/android_console.py APP [--serial SERIAL] [--mode shell|app|both] [--file PATH ...] [--dir NAME ...]
                                    [--env NAME=VALUE ...] [--expect REGEX ...] [--allow-install] [-- ARGS ...]

docs/ANDROID.md, AN0.  `python build_rs.py app APP --target android --android-abi ABI` makes
build/android/APP-<abi> (the executable) and build/android/APP-<abi>-console.apk (its test APK,
tools/android.py build_console_apk); the device's ABI picks them.

  shell  pushes the executable to /data/local/tmp/jfgui_APP/ and the --file files below it, runs it
         there from `adb shell` (SELinux domain shell, no seccomp filter), removes the folder again.
         jfgui_ANDROID_FILES / _CACHE name folders below it (lib/plat/android_files.rs).
  app    installs the test APK (package org.jflib.console.APP), copies the --file files into its files
         folder, runs the program there through `am instrument` (domain untrusted_app, Zygote's seccomp
         filter: a disallowed system call ends it with SIGSYS), reads files/console.txt, uninstalls it.
         The first line of the output says where it ran; a run outside untrusted_app or without a
         seccomp filter counts as failed.  Installing goes beyond /data/local/tmp: on a device that is
         no emulator only with --allow-install.
  both   (default) shell, then app.

ARGS go to the program (`-- --only crypto`); --file: a file the program reads by its name, at the root
of the app's files (/NAME of android_files.rs: files/fs/NAME; asynctest's --script copy.uis); --dir: an
empty folder there (copy.uis writes its shot to build/); --env: jfgui_* variables.  A run passes with exit
code 0 and every --expect found in its output.

No device: SKIP, exit code 0; no build of the device's ABI: SKIP.  Exit code 1 when a run failed.  adb
comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.  Nothing else on the device
is touched.
"""
import argparse
import base64
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INSTRUMENTATION = "org.jflib.console.ConsoleInstrumentation"


def adb_path():
    for root in (os.environ.get("ANDROID_SDK_ROOT"), os.environ.get("ANDROID_HOME"),
                 os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk"),
                 os.path.join(Path.home(), "Android", "Sdk")):
        if root:
            p = Path(root) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb")
            if p.is_file():
                return str(p)
    return shutil.which("adb") or "adb"


def package(app):
    """tools/android.py console_package"""
    return "org.jflib.console." + re.sub(r"[^a-z0-9]", "", app.lower())


def quote(word):
    """one word for the device's sh"""
    return "'" + word.replace("'", "'\\''") + "'"


class Device:
    def __init__(self, serial):
        self.adb = [adb_path()] + (["-s", serial] if serial else [])

    def run(self, *args, check=True, timeout=600, stdin=None):
        r = subprocess.run(self.adb + list(args), capture_output=True, stdin=stdin if stdin is not None else subprocess.DEVNULL,
                           timeout=timeout)
        out = r.stdout.decode("utf-8", "replace").replace("\r\n", "\n")
        err = r.stderr.decode("utf-8", "replace").replace("\r\n", "\n")
        if check and r.returncode:
            raise SystemExit(f"FAIL adb {' '.join(args)[:200]}: {out}{err}")
        return out, err, r.returncode

    def shell(self, command, **kw):
        return self.run("shell", command, **kw)

    def prop(self, name):
        return self.shell(f"getprop {name}")[0].strip()


def check(text, code, expects, label):
    """-> list of problems"""
    bad = [] if code == 0 else [f"exit code {code}"]
    bad += [f"missing /{e}/" for e in expects if not re.search(e, text, re.M)]
    return [f"{label}: {b}" for b in bad]


def run_shell(dev, app, exe, files, dirs, env, args):
    remote = f"/data/local/tmp/jfgui_{app}"
    dev.shell(f"rm -rf {remote} && mkdir -p {remote}")
    try:
        dev.run("push", str(exe), f"{remote}/{app}")
        dev.shell(f"mkdir -p {remote}/files/fs" + "".join(f" {remote}/files/fs/{quote(d)}" for d in dirs))
        for f in files:                            # the root of android_files.rs's names
            dev.run("push", str(f), f"{remote}/files/fs/{f.name}")
        dev.shell(f"chmod 755 {remote}/{app}")
        # the app's folders, which GalleryActivity and the test APK pass (lib/plat/android_files.rs)
        own = [f"jfgui_ANDROID_FILES={remote}/files", f"jfgui_ANDROID_CACHE={remote}/cache"]
        variables = " ".join(quote(e) for e in own + list(env))
        t0 = time.monotonic()
        # (the exit code through the output: older adb loses it)
        out, err, _ = dev.shell(f"cd {remote} && env {variables} ./{app} {' '.join(quote(a) for a in args)}; echo exit=$?",
                                check=False, timeout=1800)
        took = time.monotonic() - t0
    finally:
        dev.shell(f"rm -rf {remote}", check=False)
    text = out + err
    m = re.search(r"^exit=(\d+)\s*$", out, re.M)
    code = int(m.group(1)) if m else None
    return text, code, took


def run_app(dev, app, apk, files, dirs, env, args):
    pkg = package(app)
    dev.run("install", "-r", str(apk), timeout=300)
    try:
        dev.shell(f"run-as {pkg} sh -c 'mkdir -p files/fs" + "".join(f" files/fs/{d}" for d in dirs) + " && rm -f files/console.txt'")
        for f in files:
            with open(f, "rb") as data:            # stdin straight into the app's folder (no SELinux detour)
                dev.run("exec-in", "run-as", pkg, "sh", "-c", f"mkdir -p files/fs && cat > files/fs/{f.name}", stdin=data)
        extras = "".join(f" -e {key} {base64.b64encode(chr(10).join(lines).encode()).decode()}"
                         for key, lines in (("args64", args), ("env64", env)) if lines)
        command = f"am instrument -w -r{extras} {pkg}/{INSTRUMENTATION}"
        since = dev.shell("date +'%m-%d %H:%M:%S.000'")[0].strip()    # (the device's log is left as it is)
        t0 = time.monotonic()
        report, _, _ = dev.shell(command, check=False, timeout=1800)
        took = time.monotonic() - t0
        text, _, _ = dev.shell(f"run-as {pkg} cat files/console.txt", check=False)
        crash = ""
        if "Process crashed" in report or "INSTRUMENTATION_FAILED" in report or not re.search(r"^exit=-?\d+\s*$", text, re.M):
            crash = dev.run("logcat", "-d", "-b", "crash", "-T", since, check=False)[0]
            crash = "\n".join(ln for ln in crash.splitlines() if re.search(r"signal|Cause|Abort|backtrace|#0\d ", ln))[-3000:]
    finally:
        dev.run("uninstall", pkg, check=False, timeout=120)
    m = re.search(r"^exit=(-?\d+)\s*$", text, re.M)
    code = int(m.group(1)) if m else None
    if crash or code is None:
        text += "\n--- am instrument ---\n" + report.strip() + ("\n--- logcat crash ---\n" + crash if crash else "")
    where = re.search(r"^jfgui-console: context (\S+), seccomp (\w+),", text, re.M)   # (the context has commas)
    if not where or "untrusted_app" not in where.group(1) or where.group(2) != "2":
        text += f"\nFAIL: not in an app's process under seccomp ({where.group(0) if where else 'no context line'})"
        code = code if code else -1
    return text, code, took


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("app")
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--mode", choices=["shell", "app", "both"], default="both")
    ap.add_argument("--file", action="append", default=[], help="a file the program reads by its name (files/fs/NAME)")
    ap.add_argument("--dir", action="append", default=[], help="an empty folder files/fs/NAME")
    ap.add_argument("--env", action="append", default=[], help="NAME=VALUE (jfgui_* only)")
    ap.add_argument("--expect", action="append", default=[], help="a regular expression the output must contain")
    ap.add_argument("--exe", help="the executable (default: build/android/APP-<the device's ABI>)")
    ap.add_argument("--apk", help="the test APK (default: build/android/APP-<the device's ABI>-console.apk)")
    ap.add_argument("--allow-install", action="store_true", help="--mode app on a device that is no emulator")
    ap.add_argument("--quiet", action="store_true", help="only the summary lines and failures")
    ap.add_argument("args", nargs="*")
    a = ap.parse_args()
    dev = Device(a.serial)
    if dev.run("get-state", check=False)[0].strip() != "device":
        print("SKIP: no device")
        return 0
    abi = dev.prop("ro.product.cpu.abi")
    emulator = dev.prop("ro.kernel.qemu") == "1" or dev.prop("ro.boot.qemu") == "1"
    print(f"device: {abi}, {dev.prop('ro.product.model')}, Android {dev.prop('ro.build.version.release')} "
          f"(API {dev.prop('ro.build.version.sdk')}){', emulator' if emulator else ''}")
    files = [Path(f) for f in a.file]
    for d in a.dir:
        if not re.fullmatch(r"[\w.-]+(/[\w.-]+)*", d) or ".." in d.split("/"):
            raise SystemExit(f"FAIL: --dir {d}: a relative folder name")
    for f in files:
        if not f.is_file():
            raise SystemExit(f"FAIL: no file {f}")
    modes = ["shell", "app"] if a.mode == "both" else [a.mode]
    problems, ran = [], 0
    for mode in modes:
        if mode == "shell":
            exe = Path(a.exe) if a.exe else ROOT / "build" / "android" / f"{a.app}-{abi}"
            if not exe.is_file():
                print(f"SKIP shell: no {exe} (python build_rs.py app {a.app} --target android --android-abi {abi})")
                continue
            text, code, took = run_shell(dev, a.app, exe, files, a.dir, a.env, a.args)
        else:
            apk = Path(a.apk) if a.apk else ROOT / "build" / "android" / f"{a.app}-{abi}-console.apk"
            if not apk.is_file():
                print(f"SKIP app: no {apk} (python build_rs.py app {a.app} --target android --android-abi {abi})")
                continue
            if not emulator and not a.allow_install:
                print("SKIP app: installing the test APK on a device that is no emulator needs --allow-install")
                continue
            text, code, took = run_app(dev, a.app, apk, files, a.dir, a.env, a.args)
        ran += 1
        bad = check(text, code, a.expect, mode)
        print(f"--- {mode} ({took:.1f} s) ---")
        if a.quiet and not bad:
            text = "\n".join(ln for ln in text.splitlines() if re.match(r"(FAIL|SKIP|jfgui-console:|exit=|\S+: \d+ (of|passed))", ln))
        print(text.rstrip())
        problems += bad
    for p in problems:
        print(f"FAIL {p}")
    if not problems:
        print("PASS" if ran else "SKIP: nothing ran")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
