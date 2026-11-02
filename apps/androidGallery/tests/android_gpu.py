"""Android: the UI through the GPU on a device or emulator (lib/ui/gpu_out.rs, lib/plat/android_gpu.rs,
android/GalleryActivity.java; docs/GPU.md "Ausgabe unter Android").

    python apps/androidGallery/tests/android_gpu.py [--serial SERIAL] [--apk PATH]

Runs android/tests/GalleryGpuInstrumentation.java, one process per phase, and from outside what a
program cannot see of itself:
- path: first frames in the Bitmap, then the swapchain; presented images read back, the composed screen,
  partial and full frames, a menu, size changes, the keyboard where one comes, rotation requested by
  the activity, away and back, a lost device, a tool window, idle;
- images: the page "Bild & Leinwand": a canvas from tiles, a big image as a texture;
- nogpu, off: jfgui_GPU_DLL names no library / jfgui_GPU=0: the CPU path, capped resolution, it works;
- screen: the composed screen (screencap) of a page on the Bitmap and through the swapchain, upright and
  turned, full resolution and capped: the same pixels in the view's rectangle;
- guard: a run that dies while the GPU comes up (jfgui_GPU_TEST_DIE=1): the state file says so, a second
  dies too, the third start stays on the CPU path and works; a new state is written when it is cleared;
- the manifest: the APK does not require Vulkan (`aapt2 dump badging` when build-tools are there);
  gputest's window-less checks are apps/gputest/tests/android.py.

A device without a usable GPU (the emulator's -gpu swiftshader_indirect, an old phone): the phases that
need one SKIP, the fallback is still tested.  -gpu host offers Vulkan 1.3 in the emulator.
Runs on a phone unchanged: it installs the debug APK, changes no setting of the system, injects no
touch, uses only the app's own data (`pm clear` of the package; beside a gallery with data there: build
with jfgui_ANDROID_PACKAGE=NAME, tools/android.py, and pass --package NAME).  The phone must be unlocked.
Without --apk: build/android/androidGallery-<ro.product.cpu.abi>-debug.apk.  No device: SKIP, exit code 0.
Exit code 1 when something failed.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import os
import re
import shutil
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
os.environ.setdefault("MSYS_NO_PATHCONV", "1")


def adb_path():
    for root in (os.environ.get("ANDROID_SDK_ROOT"), os.environ.get("ANDROID_HOME"),
                 os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk"),
                 os.path.join(Path.home(), "Android", "Sdk")):
        if root:
            p = Path(root) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb")
            if p.is_file():
                return str(p)
    return shutil.which("adb") or "adb"


def png(data):
    """a PNG (8-bit RGB or RGBA, as screencap writes it) -> (width, height, bytes per pixel, rows)"""
    i, idat, w, h, kind = 8, b"", 0, 0, 0
    while i < len(data):
        n, t = struct.unpack(">I4s", data[i:i + 8])
        if t == b"IHDR":
            w, h, _, kind = struct.unpack(">IIBB", data[i + 8:i + 18])
        elif t == b"IDAT":
            idat += data[i + 8:i + 8 + n]
        i += 12 + n
    bpp = {2: 3, 6: 4}[kind]
    raw, stride, rows, prev, p = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp), 0
    for _ in range(h):
        f, line = raw[p], bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        prev = line
    return w, h, bpp, rows


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--apk", help="install this APK first (default: the debug build of the device's ABI)")
    ap.add_argument("--phases", default="off,nogpu,path,images,screen,guard", help="what to run")
    ap.add_argument("--package", default="org.jflib.gallery", help="the APK's package (a test build with jfgui_ANDROID_PACKAGE)")
    a = ap.parse_args()
    PKG = a.package
    RUNNER = f"{PKG}/org.jflib.gallery.GalleryGpuInstrumentation"
    ACTIVITY = f"{PKG}/org.jflib.gallery.GalleryActivity"
    adb = [adb_path()] + (["-s", a.serial] if a.serial else [])
    fails = []

    def run(*args, check=True, timeout=180):
        try:
            r = subprocess.run(adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        except subprocess.TimeoutExpired:
            if check:
                raise SystemExit(f"FAIL adb {' '.join(args)}: no answer in {timeout} s")
            return ""
        if check and r.returncode:
            raise SystemExit(f"FAIL adb {' '.join(args)}: {r.stdout}{r.stderr}")
        return r.stdout

    def ok(cond, what):
        print(("  ok   " if cond else "FAIL ") + what)
        if not cond:
            fails.append(what)
        return cond

    if run("get-state", check=False).strip() != "device":
        print("SKIP: no device")
        return 0
    abi = run("shell", "getprop", "ro.product.cpu.abi").strip()
    apk = Path(a.apk) if a.apk else ROOT / "build" / "android" / f"androidGallery-{abi}-debug.apk"
    if apk.is_file():
        print(run("install", "-r", str(apk)).strip().splitlines()[-1])
    elif run("shell", "pm", "path", PKG, check=False).strip() == "":
        print(f"SKIP: no {apk} (python build_rs.py app androidGallery --target android --android-abi {abi})")
        return 0
    state = "files/gpu-state"
    skipped = None

    def instrument(phase):
        run("shell", "am", "force-stop", PKG, check=False)
        out = run("shell", "am", "instrument", "-w", "-r", "-e", "phase", phase, RUNNER, timeout=240)
        values = dict(re.findall(r"INSTRUMENTATION_RESULT: (\w+)=(.*)", out))
        line = next((ln for ln in out.splitlines() if "PASS:" in ln or "FAIL:" in ln), out[-400:])
        return ("PASS:" in line and "INSTRUMENTATION_CODE: -1" in out), line, values

    phases = a.phases.split(",")
    view = None
    for phase in [p for p in phases if p not in ("guard", "screen")]:
        good, line, v = instrument(phase)
        view = v.get("view_rect", view)
        if v.get("skip") == "1":
            skipped = v.get("skip_why", "no usable GPU")
            print(f"SKIP {phase}: {skipped}")
            continue
        ok(good, line.strip())
        if not good and "status" in v:
            print("     status: " + v["status"])
        for key in ("view_rect", "screen_differs", "screen_differs_turned", "menu_upload", "keyboard", "rotation", "rotated_frames", "idle_frames",
                    "canvas_tiles", "image_textures", "gpu_adapter", "gpu_present_mode", "gpu_fb", "gpu_dpi", "gpu_verified", "gpu_frames",
                    "gpu_cpu_frames", "gpu_leaves", "gpu_switches", "gpu_reserved"):
            if key in v:
                print(f"       {key} = {v[key]}")

    def run_bytes(*args):
        return subprocess.run(adb + list(args), capture_output=True, timeout=60).stdout

    if "screen" in phases and not skipped:
        # the same page on both paths, from outside: what the display composes (not the status and
        # navigation bars: their contents change).  A page without animation; the keyboard's focus nowhere
        def shot(*extras):
            run("shell", "am", "force-stop", PKG, check=False)
            run("shell", "am", "start", "-W", "-n", ACTIVITY, "--es", "jfgui_LANG", "de", *extras, check=False, timeout=60)
            time.sleep(2.5)
            return png(run_bytes("exec-out", "screencap", "-p"))

        # (capped: HWUI scales the Bitmap, the compositor the surface: the same bilinear filter, rounded
        # differently, seen 1 level; 1:1 nothing may differ)
        for name, levels, extra in (("upright, full resolution", 0, ("--es", "jfgui_GPU_RES", "full")),
                                    ("upright, capped", 2, ("--es", "jfgui_GPU_RES", "cap")),
                                    ("turned, full resolution", 0, ("--es", "jfgui_GPU_RES", "full", "--es", "jfgui_TEST_LANDSCAPE", "1"))):
            a = shot("--es", "jfgui_GPU", "0", *extra)
            b = shot("--es", "jfgui_GPU", "1", *extra)
            if a[:2] != b[:2]:
                ok(False, f"the screen, {name}: sizes {a[:2]} and {b[:2]}")
                continue
            w, h, bpp, ra = a
            rb = b[3]
            # the bars: rows that differ only at the top and the bottom edge of the screen (a 7 % strip each)
            top, bottom = h * 7 // 100, h - h * 7 // 100
            if w > h:  # turned: the bars stand at the sides
                left, right = w * 7 // 100, w - w * 7 // 100
                top, bottom = h * 7 // 100, h
            else:
                left, right = 0, w
            bad, first, top_diff = 0, None, 0
            for y in range(top, bottom):
                pa, pb = ra[y][left * bpp:right * bpp], rb[y][left * bpp:right * bpp]
                if pa != pb:
                    for x in range(0, right - left):
                        if pa[x * bpp:x * bpp + 3] != pb[x * bpp:x * bpp + 3]:
                            bad += 1
                            first = first or (x + left, y)
                            top_diff = max(top_diff, max(abs(pa[x * bpp + i] - pb[x * bpp + i]) for i in range(3)))
            ok(top_diff <= levels, f"the screen, {name}: the Bitmap's and the swapchain's picture are the same ({w}x{h}; {bad} pixels differ, first at {first}, by up to {top_diff})")

    if "guard" in phases:
        # a driver that kills the program: two chances, then the CPU path until the system changes
        def alive(n=2.0):
            time.sleep(n)
            return run("shell", "pidof", PKG, check=False).strip() != ""

        def start(*extras):
            run("shell", "am", "force-stop", PKG, check=False)
            run("shell", "am", "start", "-W", "-n", ACTIVITY, *extras, check=False, timeout=60)

        def remembered():
            return run("shell", "run-as", PKG, "cat", state, check=False).strip().splitlines()[-1:] or [""]

        vulkan = "android.hardware.vulkan.version" in run("shell", "pm", "list", "features", check=False)
        run("shell", "pm", "clear", PKG, check=False)
        if not vulkan:
            print("SKIP guard: the system reports no Vulkan")
        else:
            die = ("--es", "jfgui_GPU_TEST_DIE", "1")
            start(*die)
            ok(not alive(), "a driver that kills the program: the first run dies")
            ok(remembered()[0] == "trying 1", f"the state file says trying 1: {remembered()}")
            start(*die)
            ok(not alive(), "the second run dies too")
            ok(remembered()[0] == "trying 2", f"trying 2: {remembered()}")
            start(*die)
            ok(alive(3.0), "the third run stays on the CPU path and lives")
            ok(remembered()[0] == "never", f"never: {remembered()}")
            start()
            ok(alive(3.0) and remembered()[0] == "never", "the next run too, without touching the driver")
            run("shell", "pm", "clear", PKG, check=False)
            start()
            if skipped:
                time.sleep(6)
                print(f"       no usable GPU: {remembered()}")
                ok(remembered()[0] == "never", "the state file remembers it")
            else:
                seen = ""
                for _ in range(40):
                    time.sleep(0.5)
                    seen = remembered()[0]
                    if seen == "ok":
                        break
                ok(seen == "ok", f"cleared: it tries again, the state file says ok: {seen}")
            run("shell", "pm", "clear", PKG, check=False)

    # the manifest: no phone may be refused the app because of Vulkan
    sdk = Path(adb[0]).resolve().parents[1]
    tools = sorted((sdk / "build-tools").glob("*")) if (sdk / "build-tools").is_dir() else []
    if tools and apk.is_file():
        aapt = tools[-1] / ("aapt2.exe" if os.name == "nt" else "aapt2")
        badging = subprocess.run([str(aapt), "dump", "badging", str(apk)], capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
        ok(badging != "" and "android.hardware.vulkan" not in badging, "the APK does not ask for Vulkan")
    run("shell", "am", "force-stop", PKG, check=False)
    print(f"{len(fails)} failed" if fails else "all checks passed" + (f" (no usable GPU: {skipped})" if skipped else ""))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
