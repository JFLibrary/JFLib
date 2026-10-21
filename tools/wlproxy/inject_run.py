#!/usr/bin/env python3
# inject_run.py -- (Windows side) run one wlinject.py scenario against an exe in WSL
#   python build/plat-wayland/inject_run.py SCENARIO TAG EXE [app args...]
# writes build/plat-wayland/inj_<TAG>_{app,proxy}.log and inj_<TAG>.frame
import os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DISTRO = "Ubuntu-24.04"
scen, tag, exe, args = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
d = "build/plat-wayland"
sock = f"/run/user/1000/wayland-inj-{tag}"
plog = f"{d}/inj_{tag}_proxy.log"
frame = f"{d}/inj_{tag}.frame"
proxy = subprocess.Popen(["wsl.exe", "-d", DISTRO, "--cd", ROOT.replace("\\", "/"), "-e", "python3",
                          f"{d}/wlinject.py", "/mnt/wslg/runtime-dir/wayland-0", sock, plog, "25", scen, frame],
                         stdout=subprocess.PIPE, text=True)
proxy.stdout.readline()
env = f"jfgui_BACKEND=wayland WAYLAND_DISPLAY=wayland-inj-{tag} " + os.environ.get("INJ_ENV", "")
r = subprocess.run(["wsl.exe", "-d", DISTRO, "--cd", ROOT.replace("\\", "/"), "-e", "sh", "-c",
                    f"exec env {env} ./{exe} " + " ".join(args) + " 2>&1"],
                   capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=90)
open(os.path.join(ROOT, d, f"inj_{tag}_app.log"), "w", encoding="utf-8").write(r.stdout)
try:
    proxy.wait(timeout=10)
except subprocess.TimeoutExpired:
    proxy.kill()
print(f"--- {tag}: exit {r.returncode}")
print(r.stdout.rstrip())
print("--- proxy")
print(open(os.path.join(ROOT, plog), encoding="utf-8").read().rstrip())
