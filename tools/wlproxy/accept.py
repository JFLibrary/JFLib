#!/usr/bin/env python3
# accept.py -- (Windows side) the plat-wayland acceptance runs: linuxtest's Wayland / xkb
# tests on the assembly reference and the Rust build, outputs compared.  Windowed tests run
# behind wlinject.py (the user's input and clipboard stay out).
import os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
W = ROOT.replace("\\", "/")
WSLW = "/mnt/" + W[0].lower() + W[2:]
ASM = "build/rwref/linux/linuxtest"
RS = "build/linux/linuxtest"
D = "build/plat-wayland"
fails = []


def check(ok, name, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not ok:
        fails.append(name)


def headless(exe, args):
    r = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--cd", WSLW, "-e", exe, "--shot", f"{D}/x.png"] + args,
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    return r.returncode, re.sub(r"in \d+ us", "in T us", r.stdout).replace(f"{D}/rs_", f"{D}/X_").replace(f"{D}/asm_", f"{D}/X_")


def injected(scen, tag, exe, args):
    subprocess.run([sys.executable, f"{D}/inject_run.py", scen, tag, exe] + args, capture_output=True, timeout=200,
                   cwd=ROOT)
    app = open(os.path.join(ROOT, D, f"inj_{tag}_app.log"), encoding="utf-8").read()
    proxy = open(os.path.join(ROOT, D, f"inj_{tag}_proxy.log"), encoding="utf-8").read()
    proxy = "\n".join(l for l in proxy.splitlines() if not l.startswith("frame "))
    return app, proxy


# ---- headless: synth, cursors (PNG bytes), xkb (+ dump)
for name, args in (("synth", ["--test", "synth"]),
                   ("xkb", ["--test", "xkb", "--keymap", f"{D}/keymap.xkb", "--xkbdump"])):
    a, b = headless(ASM, args), headless(RS, args)
    check(a == b and a[0] == 0, f"linuxtest --test {name}: exit code and stdout identical", f"exit {a[0]} / {b[0]}")
a = headless(ASM, ["--test", "cursors", "--out", f"{D}/asm_cur.png"])
b = headless(RS, ["--test", "cursors", "--out", f"{D}/rs_cur.png"])
same = open(os.path.join(ROOT, D, "asm_cur.png"), "rb").read() == open(os.path.join(ROOT, D, "rs_cur.png"), "rb").read()
check(a == b and same, "linuxtest --test cursors: stdout identical, cursor sheet PNG byte-identical")

# ---- windowed, behind the proxy
runs = [("none", "window", ["--test", "window"]),
        ("none", "show", ["--test", "show", "--hold", "1500"]),
        ("none", "protoerr", ["--test", "protoerr"]),
        ("grab", "grab", ["--test", "grab", "--hold", "3000"]),
        ("clip", "clip", ["--test", "clip", "--hold", "2500"]),
        ("input", "input", ["--log-events", "--auto-close", "5000"]),
        ("drop", "drop", ["--log-events", "--auto-close", "3500"])]
for scen, name, args in runs:
    ea, pa = injected(scen, name + "_asm", ASM, args)
    eb, pb = injected(scen, name + "_rs", RS, args)
    if scen in ("input", "drop"):
        # the assembly app is the UI runtime (it repaints its frame); compare the events
        ea = "\n".join(l for l in ea.splitlines() if l.startswith("event"))
        eb = "\n".join(l for l in eb.splitlines() if l.startswith("event"))
    check(ea == eb, f"wayland {name}: app output identical", f"{len(ea.splitlines())} lines")
    check(pa == pb, f"wayland {name}: proxy log identical (requests to the fake devices / clients)")

# anim: the last committed frame equals the saved framebuffer (both builds)
for tag, exe in (("anim_asm", ASM), ("anim_rs", RS)):
    injected("none", tag, exe, ["--test", "anim", "--hold", "1500", "--save", f"{D}/{tag}.png", "--after", "800"])
    r = subprocess.run([sys.executable, f"{D}/framecmp.py", f"{D}/inj_{tag}.frame", f"{D}/{tag}.png"], cwd=ROOT,
                       capture_output=True, text=True)
    check(r.returncode == 0, f"wayland anim ({tag[5:]}): the last committed frame is the framebuffer", r.stdout.strip())

print("accept:", "all passed" if not fails else f"{len(fails)} failed")
sys.exit(1 if fails else 0)
