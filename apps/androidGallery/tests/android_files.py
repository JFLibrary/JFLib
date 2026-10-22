"""Android: files, the documents of other apps (Storage Access Framework) and the system clipboard
on a device or emulator (android/tests/GalleryFilesInstrumentation.java, apps/androidGallery/files.rs).

    python apps/androidGallery/tests/android_files.py [--serial emulator-5554] [--apk PATH]

Puts /sdcard/Documents/jfgui-test/hello.txt in place, clears the gallery's data (and with it its
grants), runs phase 1 (the self-test, the picker answered through UiAutomation; content://
saved over existing documents, the name cache, a reported change, a write interrupted after six
bytes), stops the app, runs phase 2 (a new process: files and grants after the restart, the
interrupted write finished from the journal, paste / copy in a text box).  After each phase the
saved document and the app's journal folder are looked at from outside (cat, run-as ls).
The APK must be a debug build (the self-test reads jfgui_* extras only there).  Exit code 0 when
both phases pass.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

PKG = "org.jflib.gallery"
RUNNER = f"{PKG}/.GalleryFilesInstrumentation"
DIR = "/sdcard/Documents/jfgui-test"


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
        return r.stdout

    if a.apk:
        print(run("install", "-r", a.apk).strip().splitlines()[-1])
    run("shell", f"rm -rf {DIR} && mkdir -p {DIR}")
    with tempfile.TemporaryDirectory() as d:
        hello = Path(d) / "hello.txt"
        hello.write_bytes(b"Hallo vom Host\n")
        run("push", str(hello), f"{DIR}/hello.txt")
    run("shell", "pm", "clear", PKG)
    failures = 0
    for phase in ("1", "2"):
        out = run("shell", "am", "instrument", "-w", "-r", "-e", "phase", phase, RUNNER, check=False, timeout=300)
        ok = "INSTRUMENTATION_CODE: -1" in out and "stream=PASS" in out
        stream = out[out.find("stream="):].split("INSTRUMENTATION_CODE")[0] if "stream=" in out else out
        print(f"phase {phase}: {'PASS' if ok else 'FAIL'}\n{stream.strip()}\n", flush=True)
        failures += not ok
        run("shell", "am", "force-stop", PKG)
        # a picker left open by a failed phase belongs to DocumentsUI's process
        run("shell", "am", "force-stop", "com.android.documentsui", check=False)
        # the write phase 1 interrupted: a part in the document, the whole in the app's journal
        # (lib/plat/content.rs); phase 2 finished it when content:// was first used
        journal = run("shell", "run-as", PKG, "ls", "files/fs/data/androidGallery/.content-journal", check=False).split()
        saved = run("shell", f"cat {DIR}/gespeichert.txt", check=False)
        want = ("vollst", 1) if phase == "1" else ("vollständig nach dem Neustart geschrieben", 0)
        good = saved == want[0] and len([j for j in journal if j.endswith(".jw")]) == want[1]
        print(f"after phase {phase}: gespeichert.txt {saved!r}, journal {journal}: {'PASS' if good else 'FAIL'}", flush=True)
        failures += not good
    left = run("shell", f"ls {DIR}", check=False).split()
    print("Documents/jfgui-test:", " ".join(left))
    print("android_files:", "ok" if failures == 0 else f"{failures} phase(s) failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
