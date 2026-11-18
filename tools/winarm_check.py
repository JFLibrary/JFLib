"""ARM5 (docs/ARM64.md): the Windows programs for ARM64 on a Windows ARM64 machine (the runner windows-11-arm).
They are built beforehand in build/win-arm64 (on an x64 PC or runner: build_rs.py app NAME --target win --arch
aarch64), so the machine needs no compiler.  .github/workflows/arm64-windows.yml runs this there.

    python tools/winarm_check.py [--target win|linux] [--dir DIR] [--other-module FILE]
                                 [--only scripts|crash|module|gpu] [--gallery-all] [--scripts APP[/SCRIPT.uis],...]

--target linux (ARM8): the Linux programs of build/linux-arm64 on an ARM64 Linux machine (ubuntu-24.04-arm, the
ARM64 VM), with a display of its own (WAYLAND_DISPLAY or DISPLAY); scripts, crash and gpu as below, no module.

    scripts  every script of coretest and plattest, and a start of the gallery (basic.uis, with a picture;
             --gallery-all: every script of the gallery; --scripts: these apps' scripts instead, an app with
             all of them, APP/SCRIPT.uis one; ARM8 runs gallery, jfWord, gputest and uigputest so)
    crash    what no script runs: plattest --test crash, guardcrash and panic end in the crash handler with
             exit code 3 and no FAIL (the CONTEXT of ARM64, RtlVirtualUnwind, the trampoline's unwind data)
    module   codectest with the module jxlenc: its files equal the linked writer's byte for byte, a panic and
             a fault inside the module end with exit code 3 and a report naming it; --other-module FILE (the
             x64 build of jxlenc.dll) must be refused with LIBE_ARCH while the linked writer saves
    gpu      (ARM8) what the machine offers the GPU: gputest --info and a start of uigputest's output with
             jfgui_GPU_TRACE=1 (what the UI does without vulkan-1.dll or without a driver: the CPU path and
             the reason); both end with exit code 0, their lines are printed

Exit code 1 when something failed."""
import argparse
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import build_rs
from tools import host_exec

ROOT = build_rs.ROOT
SCRIPTS = {"coretest": None, "plattest": None, "gallery": ["basic.uis"]}   # None: every script (--gallery-all)
TARGET = "win"          # --target


def prog(d, name):
    """the program name of folder d (Windows: name.exe)"""
    return d / (f"{name}.exe" if TARGET == "win" else name)
FIXTURES = ROOT / "apps" / "codectest" / "fixtures"


class Tally:
    def __init__(self):
        self.passed = self.failed = 0

    def check(self, ok, name, detail=""):
        if ok:
            self.passed += 1
        else:
            self.failed += 1
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)


def run(exe, args, timeout=600, env=None):
    """exit code (124: no end within timeout) and everything the program wrote"""
    e = dict(os.environ, **(env or {}))
    try:
        r = subprocess.run([str(exe), *map(str, args)], cwd=ROOT, timeout=timeout, env=e, capture_output=True,
                           text=True, encoding="utf-8", errors="replace", creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired as x:
        out = (x.stdout or b"").decode("utf-8", "replace") if isinstance(x.stdout, bytes) else (x.stdout or "")
        return 124, out + f"\nno end within {timeout} s"


def scripts(t, d):
    run_native = build_rs.run_exe

    def run_exe(target, exe, args, timeout=600):     # a script that hangs fails, the ones after it still run
        try:
            return run_native(target, exe, args, timeout)
        except subprocess.TimeoutExpired:
            return subprocess.CompletedProcess(args, 124, "", f"FAIL no end within {timeout} s\n")

    build_rs.run_exe = run_exe
    for app, only in SCRIPTS.items():
        exe = prog(d, app)
        records = build_rs.run_app_tests(app, TARGET, exe) if only is None else [one_script(app, exe, s) for s in only]
        for r in records:
            if r["status"] == "skipped":
                continue
            bad = [ln for ln in r.get("output", "").splitlines() if not ln.startswith("PASS")]
            t.check(r["status"] == "passed", f"{app}/{r['script']} ({r['duration']:.1f} s)",
                    "" if r["status"] == "passed" else f"exit {r['exit_code']}\n   " + "\n   ".join(bad[-40:]))


def one_script(app, exe, name):
    """one script of app, as build_rs.run_app_tests runs each (its #!args, its timeout, the folders of its pictures)"""
    script = ROOT / "apps" / app / "tests" / name
    for line in script.read_text(encoding="utf-8").splitlines():
        if line.startswith(("shot ", "dump ")) and len(line.split()) > 1:
            (ROOT / line.split()[1]).parent.mkdir(parents=True, exist_ok=True)
    args = build_rs.script_args(script) + ["--script", script.relative_to(ROOT).as_posix()]
    started = time.monotonic()
    r = build_rs.run_exe(TARGET, exe, args, timeout=build_rs.script_timeout(script))
    return dict(script=name, status="passed" if r.returncode == 0 else "failed", exit_code=r.returncode,
                duration=time.monotonic() - started, output=r.stdout + r.stderr)


def gpu(t, d):
    """gputest --info, then uigputest's output starting (--test switch, jfgui_GPU_TRACE=1): what the machine has and
    what the UI does with it"""
    code, out = run(prog(d, "gputest"), ["--info"], timeout=300)
    lines = [ln.rstrip() for ln in out.splitlines() if ln.strip()]
    t.check(code == 0 and "FAIL" not in out, f"gputest --info: exit {code}", "\n   ".join(lines[:40]))
    code, out = run(prog(d, "uigputest"), ["--test", "switch"], timeout=300, env={"jfgui_GPU_TRACE": "1"})
    lines = [ln.rstrip() for ln in out.splitlines() if ln.startswith(("SKIP", "FAIL", "gpu:", "  ok", "jfgui:", "all checks"))]
    t.check(code == 0, f"uigputest --test switch with jfgui_GPU_TRACE=1: exit {code}", "\n   ".join(lines[:40]))


def crash(t, d):
    exe = prog(d, "plattest")
    for test in ("crash", "guardcrash", "panic"):
        code, out = run(exe, ["--test", test], timeout=300)
        fails = [ln for ln in out.splitlines() if ln.startswith("FAIL")]
        info = [ln.strip() for ln in out.splitlines() if ln.startswith(("  (", "jfgui:"))]
        t.check(code == 3 and not fails and "0 failure(s)" in out, f"plattest --test {test}: exit {code}",
                "\n   ".join(info + fails) if code != 3 or fails else " / ".join(info[:3]))


def module(t, d, other):
    exe, mod = d / "codectest.exe", d / "jxlenc.dll"
    if not mod.is_file():
        t.check(False, "jxlenc: the module", f"{mod} is missing")
        return
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for f in ("a1_rgba", "a6_photo"):
            for q in ([], ["--quality", "90"]):
                src = FIXTURES / f"{f}.png"
                a, out_a = run(exe, ["--convert-ex", src, "--to", tmp / "lin.jxl", *q])
                b, out_b = run(exe, ["--convert-ex", src, "--to", tmp / "mod.jxl", *q, "--jxlenc", mod])
                lin, m = (tmp / "lin.jxl"), (tmp / "mod.jxl")
                same = a == 0 and b == 0 and lin.is_file() and m.is_file() and lin.read_bytes() == m.read_bytes()
                size = m.stat().st_size if m.is_file() else 0
                t.check(same, f"jxlenc: {f} {' '.join(q) or 'lossless'}: the module's file = the linked writer's ({size} bytes)",
                        "" if same else f"exit {a} / {b}\n   {out_a.strip()[:300]}\n   {out_b.strip()[:300]}")
                for p in (lin, m):
                    p.unlink(missing_ok=True)
        for k in ("panic", "fault"):
            c, out = run(exe, ["--convert-ex", FIXTURES / "a1_rgba.png", "--to", tmp / "x.jxl", "--jxlenc", mod],
                         env={"jfgui_JXLENC_TEST": k})
            hit = [ln for ln in out.splitlines() if "panic: jxlenc" in ln or "in jxlenc" in ln]
            t.check(c == 3 and bool(hit), f"jxlenc: a {k} inside the module: exit {c}", (hit or [out.strip()[:400]])[0])
        if other:
            c, out = run(exe, ["--convert-ex", FIXTURES / "a1_rgba.png", "--to", tmp / "y.jxl", "--jxlenc", other])
            line = next((ln for ln in out.splitlines() if "jfgui" in ln), out.strip()[:300])
            t.check(c == 0 and "another processor" in out and (tmp / "y.jxl").is_file(),
                    f"jxlenc: the other processor's module is refused (LIBE_ARCH), the linked writer saves: exit {c}", line)


def main():
    global TARGET
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--target", choices=("win", "linux"), default="win", help="the programs' system (linux: ARM8)")
    ap.add_argument("--dir", help="the built programs (default build/win-arm64, build/linux-arm64)")
    ap.add_argument("--other-module", help="jxlenc.dll built for x64 (must be refused)")
    ap.add_argument("--only", choices=("scripts", "crash", "module", "gpu"))
    ap.add_argument("--gallery-all", action="store_true", help="every script of the gallery, not only a start")
    ap.add_argument("--scripts", help="APP or APP/SCRIPT.uis, separated by commas: these scripts instead of the default")
    a = ap.parse_args()
    if a.gallery_all:
        SCRIPTS["gallery"] = None
    if a.scripts:
        SCRIPTS.clear()
        for name in a.scripts.split(","):
            app, _, one = name.strip().partition("/")
            if one:
                SCRIPTS.setdefault(app, []).append(one)
            else:
                SCRIPTS[app] = None
    TARGET = a.target
    d = Path(a.dir or ROOT / "build" / ("win-arm64" if TARGET == "win" else "linux-arm64"))
    first = next((prog(d, n) for n in [*SCRIPTS, "plattest"] if prog(d, n).is_file()), prog(d, "plattest"))
    print(f"host {host_exec.host_arch()}, programs in {d} for {host_exec.exe_arch(first)}", flush=True)
    t = Tally()
    if a.only in (None, "crash"):
        crash(t, d)
    if a.only == "module" or a.only is None and TARGET == "win":
        module(t, d, a.other_module)
    if a.only == "gpu" or a.only is None and prog(d, "gputest").is_file():     # (ARM5's runs built no gputest)
        gpu(t, d)
    if a.only in (None, "scripts"):
        scripts(t, d)
    print(f"winarm_check: {t.passed} of {t.passed + t.failed} passed")
    return 1 if t.failed else 0


if __name__ == "__main__":
    sys.exit(main())
