"""ARM0's side test (docs/ARM64.md): the scripts of coretest, plattest and gallery on programs that are
already built in build/win, so the machine needs no compiler. .github/workflows/arm64-emulation.yml runs
it on x64 and on windows-11-arm.

    python tools/emulation_check.py [APP ...]

Exit code 1 when a script failed."""
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import build_rs

APPS = ("coretest", "plattest", "gallery")
run_native = build_rs.run_exe


def run_exe(target, exe, args, timeout=600):
    """a script that hangs counts as failed, and the ones after it still run"""
    try:
        return run_native(target, exe, args, timeout)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(args, 124, "", f"FAIL no end within {timeout} s\n")


def main():
    build_rs.run_exe = run_exe
    passed = failed = 0
    for app in sys.argv[1:] or APPS:
        exe = build_rs.ROOT / "build" / "win" / f"{app}.exe"
        for r in build_rs.run_app_tests(app, "win", exe):
            if r["status"] == "skipped":
                continue
            passed += r["status"] == "passed"
            failed += r["status"] == "failed"
            print(f"{r['status'].upper()} {app}/{r['script']} ({r['duration']:.1f} s)")
            if r["status"] == "failed":
                print(f"   exit {r['exit_code']}")
                for ln in r["output"].splitlines():
                    if not ln.startswith("PASS"):
                        print("   " + ln)
    print(f"{passed} of {passed + failed} scripts passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
