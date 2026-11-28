"""jfWord on an Android device or emulator: the self-test and headless scripts per `adb shell` and in an
app's process (docs/ANDROID.md, AN5; tools/android_console.py does each run).

    jfgui_ANDROID_CONSOLE=1 python build_rs.py app jfWord --target android --android-abi ABI
    python apps/jfWord/tests/android.py [--serial SERIAL] [--mode shell|app|both] [--allow-install]

The build makes build/android/jfWord-<abi> and its test APK instead of jfWord's own APK (build_rs.py,
jfgui_ANDROID_CONSOLE).  Each run starts in empty folders: a script that replaces a file of an earlier
run in the suite's build/ gets an empty one (--touch).  The UI on the phone itself (the system's picker,
the print dialog) is a hand test, docs/ANDROID.md AN5.  No device or no build: SKIP, exit code 0.
"""
import argparse
import shlex
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TESTS = Path(__file__).resolve().parent

# (what, the runner's options, the program's arguments)
RUNS = [
    ("self-test", ["--dir", "build"], ["--self-test"]),
    ("files.uis", ["--touch", "build/jfword-ui-roundtrip.docx"], None),
    ("editor.uis", ["--touch", "build/jfword-editor.docx"], None),
    ("print.uis", [], None),
]


def script_args(uis):
    first = (TESTS / uis).read_text(encoding="utf-8").splitlines()[:1]
    return shlex.split(first[0][len("#!args"):]) if first and first[0].startswith("#!args") else []


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial")
    ap.add_argument("--mode", choices=["shell", "app", "both"], default="both")
    ap.add_argument("--allow-install", action="store_true")
    ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args()
    failed = []
    for what, options, args in RUNS:
        command = [sys.executable, str(ROOT / "tools" / "android_console.py"), "jfWord", "--mode", a.mode, *options]
        if args is None:                           # a script: copied to the app's files, its #!args
            command += ["--file", str(TESTS / what), "--dir", "build"]
            args = ["--script", what, *script_args(what)]
        if a.serial:
            command += ["--serial", a.serial]
        if a.allow_install:
            command.append("--allow-install")
        if a.quiet:
            command.append("--quiet")
        r = subprocess.run(command + ["--", *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                           errors="replace", stdin=subprocess.DEVNULL)
        out = (r.stdout + r.stderr).strip()
        print(f"=== {what}\n{out}")
        if r.returncode:
            failed.append(what)
        elif "SKIP: no device" in out:
            return 0
    print(f"FAIL {', '.join(failed)}" if failed else "PASS")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
