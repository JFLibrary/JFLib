"""Android: the motion sensors and the location on a device or emulator (android/GallerySensors.java,
lib/plat/sensors.rs, apps/androidGallery/pg_sensors.rs), and the pages' touch: the back key, a long
press.

    python apps/androidGallery/tests/android_sensors.py [--serial SERIAL] [--apk PATH]

Starts the gallery with jfgui_SELFTEST=sensors and jfgui_PAGE=level|attitude|compass|gps (every change
in data://sensors-state.txt, read with run-as) and works it as a user would (taps found in
`uiautomator dump`):
- the sensors run only while their page is open: states, plausible values (gravity about 9.8 m/s^2
  while the phone lies still, heading 0 to 360), the background pauses them (`input keyevent
  KEYCODE_HOME`, `dumpsys sensorservice`), back again they run; a sensor the device lacks says
  "unavailable", which is no failure;
- the location: nothing before the switch; the system's permission dialog: refused -> denied, allowed
  -> searching, a fix within FIX_TIMEOUT s, or a clear "no fix (indoors?)" that is no failure; the
  system's location switch off -> disabled, on again; the background; a permission refused for good
  (no dialog) -> denied;
- the back key on a page goes to the start page, on the start page it leaves the app; a long press
  opens a context menu.
On an emulator (`adb emu sensor set`, `adb emu geo fix`) the values are also compared with what was
set.  On a phone the script prints what the person should do ("Telefon flach hinlegen") and waits.
Everything it changes is restored at the end (also after a failure or Ctrl+C): the two location
permissions and their flags, the system's location switch, the emulator's sensor values.  The phone
must be unlocked (a screen lock with a PIN cannot be dismissed from adb).  The APK must be a debug
build (jfgui_* extras and run-as); without --apk it installs
build/android/androidGallery-<ro.product.cpu.abi>-debug.apk when that exists.  Exit code 0 when
everything passes.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
import math
import os
import re
import shutil
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from pathlib import Path

PKG = "org.jflib.gallery"
ACTIVITY = f"{PKG}/.GalleryActivity"
STATE = "files/fs/data/androidGallery/sensors-state.txt"
FINE = "android.permission.ACCESS_FINE_LOCATION"
COARSE = "android.permission.ACCESS_COARSE_LOCATION"
DUMP = "/data/local/tmp/jfgui_sensors_ui.xml"
ROOT = Path(__file__).resolve().parents[3]
FIX_TIMEOUT = 60
G = 9.81


def adb_path():
    for root in (os.environ.get("ANDROID_SDK_ROOT"), os.environ.get("ANDROID_HOME"),
                 os.path.join(os.environ.get("LOCALAPPDATA", ""), "Android", "Sdk"),
                 os.path.join(Path.home(), "Android", "Sdk")):
        if root:
            p = Path(root) / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb")
            if p.is_file():
                return str(p)
    return shutil.which("adb") or "adb"


class Device:
    def __init__(self, serial):
        self.adb = [adb_path()] + (["-s", serial] if serial else [])

    def run(self, *args, check=True, timeout=120):
        try:
            r = subprocess.run(self.adb + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace",
                               timeout=timeout)
        except subprocess.TimeoutExpired:
            if check:
                raise RuntimeError(f"adb {' '.join(args)}: no answer in {timeout} s")
            return ""
        if check and r.returncode != 0:
            raise RuntimeError(f"adb {' '.join(args)}: {r.stdout}{r.stderr}")
        return r.stdout

    def sh(self, *args, check=False, timeout=120):
        return self.run("shell", *args, check=check, timeout=timeout)

    def prop(self, name):
        return self.sh("getprop", name).strip()


def vec(text):
    return [float(x) for x in text.split()]


def norm(v):
    return math.sqrt(sum(x * x for x in v[:3]))


def angle_diff(a, b):
    return abs((a - b + 180) % 360 - 180)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--apk", help="install this APK first (default: build/android/androidGallery-<abi>-debug.apk if it exists)")
    a = ap.parse_args()
    d = Device(a.serial)
    sdk = int(d.prop("ro.build.version.sdk") or "0")
    abi = d.prop("ro.product.cpu.abi")
    emulator = (a.serial or "").startswith("emulator-") or d.prop("ro.kernel.qemu") == "1" or d.prop("ro.boot.qemu") == "1"
    print(f"device: {d.prop('ro.product.model')} (API {sdk}, {abi}){' emulator' if emulator else ''}", flush=True)
    apk = Path(a.apk) if a.apk else ROOT / "build" / "android" / f"androidGallery-{abi}-debug.apk"
    if apk.is_dir():
        apk = apk / f"androidGallery-{abi}-debug.apk"
    if a.apk or apk.is_file():
        if not apk.is_file():
            sys.exit(f"no APK {apk} (build it: python build_rs.py app androidGallery --target android --android-abi {abi})")
        print(d.run("install", "-r", str(apk), timeout=300).strip().splitlines()[-1], flush=True)

    failures = []
    checks = 0
    notes = []

    def ok(cond, what):
        nonlocal checks
        print(f"  {'ok  ' if cond else 'FAIL'} {what}", flush=True)
        if cond:
            checks += 1
        else:
            failures.append(what)
        return cond

    def skip(what):
        print(f"  skip {what}", flush=True)
        notes.append(what)

    def ask_person(what, seconds=12, watch=(), label=""):
        """(a phone) what the person should do; an emulator has no person.  With `watch` (angles
        in the state): whether they moved meanwhile, which is a note, not a failure"""
        if emulator:
            return
        print(f"\n  >>> Bitte jetzt: {what} ({seconds} s Zeit)", flush=True)
        first, moved = {}, 0.0
        end = time.time() + seconds
        while time.time() < end:
            s = state()
            for key in watch:
                try:
                    v = float(s[key])
                except (KeyError, ValueError):
                    continue
                moved = max(moved, angle_diff(v, first.setdefault(key, v)))
            time.sleep(0.4)
        if watch and moved >= 15:
            ok(True, f"{label} follows the phone: {moved:.0f} degrees")
        elif watch:
            skip(f"{label}: only {moved:.0f} degrees of movement seen (was the phone moved?)")

    # ---------------------------------------------------------------- what is restored
    def perm_state(perm):
        m = re.search(re.escape(perm) + r": granted=(\w+), flags=\[([^\]]*)\]", d.sh("dumpsys", "package", PKG))
        return (m.group(1) == "true", m.group(2)) if m else None

    def location_enabled():
        if sdk >= 30:
            return "true" in d.sh("cmd", "location", "is-location-enabled")
        return d.sh("settings", "get", "secure", "location_mode").strip() not in ("0", "null", "")

    def location_set(on):
        if sdk >= 31:
            d.sh("cmd", "location", "set-location-enabled", "true" if on else "false")
        else:
            d.sh("settings", "put", "secure", "location_mode", "3" if on else "0")

    saved_perms = {p: perm_state(p) for p in (FINE, COARSE)}
    saved_location = location_enabled()
    emu_sensors = ("acceleration", "magnetic-field", "gyroscope")
    saved_emu = {}
    if emulator:
        for name in emu_sensors:
            m = re.search(rf"{name} = ([-\d.:e]+)", d.run("emu", "sensor", "get", name, check=False))
            if m:
                saved_emu[name] = m.group(1)

    def emu_sensor(name, *values):
        d.run("emu", "sensor", "set", name, ":".join(str(v) for v in values), check=False)

    def perm_set(perm, granted, flags=()):
        d.sh("pm", "grant" if granted else "revoke", PKG, perm)
        d.sh("pm", "clear-permission-flags", PKG, perm, "user-set", "user-fixed")
        if flags:
            d.sh("pm", "set-permission-flags", PKG, perm, *flags)

    # what the person sees of a permission: given or not, asked again or not (the system keeps a
    # flag of its own about the accuracy once it was asked; that one means nothing while refused)
    def perm_seen(state):
        return state and (state[0], "USER_SET" in state[1], "USER_FIXED" in state[1])

    def no_dialog():
        # a permission dialog left open (a failed run) stays on top of the app's task
        for _ in range(3):
            focus = "".join(l for l in d.sh("dumpsys", "window").splitlines() if "mCurrentFocus" in l)
            if "GrantPermissionsActivity" not in focus:
                return
            d.sh("input", "keyevent", "KEYCODE_BACK")
            time.sleep(1)

    def restore():
        no_dialog()
        d.sh("am", "force-stop", PKG)
        # the two permissions are one group for the system: changing one touches the other's flags,
        # so all of them twice
        for _ in range(2):
            for perm, was in saved_perms.items():
                if was is not None:
                    granted, flags = was
                    perm_set(perm, granted, [f.lower().replace("_", "-") for f in ("USER_SET", "USER_FIXED") if f in flags])
        if location_enabled() != saved_location:
            location_set(saved_location)
        for name, value in saved_emu.items():
            d.run("emu", "sensor", "set", name, value, check=False)
        d.run("shell", "run-as", PKG, "rm", "-f", STATE, check=False)
        d.sh("rm", "-f", DUMP)

    # ---------------------------------------------------------------- the app's state, the screen
    def state():
        text = d.run("shell", "run-as", PKG, "cat", STATE, check=False)
        return dict(line.split("=", 1) for line in text.splitlines() if "=" in line)

    def wait(pred, what, timeout=15):
        end = time.time() + timeout
        s = {}
        while time.time() < end:
            s = state()
            try:
                if s and pred(s):
                    return s
            except (KeyError, ValueError, IndexError):
                pass
            time.sleep(0.25)
        print(f"    ({what}: state: {' '.join(f'{k}={v}' for k, v in s.items())})")
        return None

    def awake():
        d.sh("input", "keyevent", "KEYCODE_WAKEUP")
        d.sh("wm", "dismiss-keyguard")

    def nodes():
        d.sh("rm", "-f", DUMP)
        d.sh("uiautomator", "dump", DUMP, timeout=60)
        xml = d.run("exec-out", "cat", DUMP, check=False)
        start = xml.find("<?xml")
        if start < 0:
            return []
        try:
            root = ET.fromstring(xml[start:xml.rfind(">") + 1])
        except ET.ParseError:
            return []
        return list(root.iter("node"))

    def find(match, what, timeout=20):
        end = time.time() + timeout
        while time.time() < end:
            for n in nodes():
                if match(n):
                    return n
            time.sleep(0.5)
        print(f"    (nothing on the screen for {what})")
        return None

    def center(n):
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", n.get("bounds", "[0,0][0,0]")))
        return (x1 + x2) // 2, (y1 + y2) // 2

    def tap(n):
        x, y = center(n)
        d.sh("input", "tap", str(x), str(y))

    def label(text):
        return lambda n: text in (n.get("text"), n.get("content-desc"))

    def tap_label(text):
        awake()
        n = find(label(text), f'"{text}"')
        if n is None:
            return False
        tap(n)
        return True

    def launch(extra=()):
        d.sh("am", "start", "-W", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER",
             "-n", ACTIVITY, *extra, timeout=60)

    def start(page):
        awake()
        no_dialog()
        d.sh("am", "force-stop", PKG)
        d.run("shell", "run-as", PKG, "rm", "-f", STATE, check=False)
        launch(["--es", "jfgui_LANG", "de", "--es", "jfgui_SELFTEST", "sensors", "--es", "jfgui_PAGE", page])
        s = wait(lambda s: s.get("selftest") == "sensors" and s.get("page", "").lower() == page, f"page {page}", 30)
        if not ok(s is not None, f"the gallery runs on the page {page}"):
            raise RuntimeError("the app did not start (a debug APK?)")
        return s

    def go_home():
        d.sh("input", "keyevent", "KEYCODE_HOME")

    def foreground():
        return f"{PKG}/" in "".join(line for line in d.sh("dumpsys", "window").splitlines() if "mCurrentFocus" in line)

    # the listeners of our package in the sensor service (None: a system that does not say)
    def sensor_connection():
        out = d.sh("dumpsys", "sensorservice")
        if "active connections" not in out:
            return None
        return PKG in out.split("active connections", 1)[-1].split("Previous Registrations", 1)[0]

    def sensor_state(s, name):
        return s.get(f"{name}.state", "")

    # the buttons of the system's permission dialog; "while using" is the foreground grant
    ALLOW = ("permission_allow_foreground_only_button", "permission_allow_button")
    DENY = ("permission_deny_button", "permission_deny_and_dont_ask_again_button")

    def dialog(buttons, what):
        is_button = lambda x: any(x.get("resource-id", "").endswith(b) for b in buttons)
        n = find(is_button, what, 12)
        if n is None:
            return False
        for _ in range(4):
            time.sleep(0.8)
            tap(n)
            time.sleep(1.0)
            n = next((x for x in nodes() if is_button(x)), None)
            if n is None:
                return True
        return False

    try:
        # ------------------------------------------------------------ the level: gravity
        print("Wasserwaage:", flush=True)
        s = start("level")
        s = wait(lambda s: sensor_state(s, "accel") in ("running", "unavailable"), "the accelerometer answers")
        ok(s is not None, f"the accelerometer: {sensor_state(s or {}, 'accel')}")
        ok(sensor_state(s or {}, "gyro") == "off", "the level does not start the gyroscope")
        ok(sensor_state(s or {}, "magnet") == "off", "... nor the magnetometer")
        ok((s or {}).get("loc.state") == "off", "... nor the location")
        if sensor_state(s or {}, "accel") == "running":
            ok(sensor_connection() is not False, "the sensor service lists the app's connection")
            if emulator:
                emu_sensor("acceleration", 0, 0, G)
            ask_person("Telefon flach und still auf den Tisch legen, Bildschirm nach oben")
            s = wait(lambda s: abs(norm(vec(s["accel"])) - G) < 1.0 and vec(s["accel"])[2] > 7, "lying flat", 20)
            if ok(s is not None, f"gravity about 9.8 m/s^2 on z while it lies flat: accel {(s or {}).get('accel')}"):
                print(f"    gravity sensor: {sensor_state(s, 'gravity')}, rotation estimate: {sensor_state(s, 'rot')}")
            # pitch and roll follow (gravity or the rotation estimate; the system's fusion needs a moment)
            s = wait(lambda s: abs(float(s["pitch"])) < 10 and abs(float(s["roll"])) < 10, "pitch and roll flat", 25)
            ok(s is not None, f"pitch and roll about 0: {(s or {}).get('pitch')} {(s or {}).get('roll')}")
            if emulator:
                # upright: the accelerometer says so at once; a fused estimate may follow later
                emu_sensor("acceleration", 0, G, 0)
                s = wait(lambda s: vec(s["accel"])[1] > 9, "upright: accel y")
                ok(s is not None, "emu sensor set acceleration 0:9.81:0 reaches the app")
                s = wait(lambda s: float(s["pitch"]) > 70, "pitch up", 25)
                ok(s is not None, f"pitch follows: {(s or {}).get('pitch')}")
                emu_sensor("acceleration", 0, 0, G)
        else:
            skip("no accelerometer: no gravity to check")

        # the background stops them
        go_home()
        s = wait(lambda s: s.get("suspend") == "2" and sensor_state(s, "accel") in ("paused",), "paused in the background")
        ok(s is not None, "Home: UIC_SUSPEND 2, the sensors paused")
        if sensor_state(s or {}, "accel") == "paused":
            time.sleep(1)
            ok(sensor_connection() is not True, "... and the sensor service no longer lists the app's listeners")
        launch()
        s = wait(lambda s: s.get("suspend") == "0" and sensor_state(s, "accel") in ("running", "waiting", "unavailable"), "running again")
        ok(s is not None, f"back: UIC_SUSPEND 0, the accelerometer: {sensor_state(s or {}, 'accel')}")

        # ------------------------------------------------------------ the compass
        print("Kompass:", flush=True)
        s = start("compass")
        s = wait(lambda s: sensor_state(s, "magnet") in ("running", "unavailable"), "the magnetometer answers")
        ok(s is not None, f"the magnetometer: {sensor_state(s or {}, 'magnet')}")
        if sensor_state(s or {}, "magnet") == "running" or sensor_state(s or {}, "rot") == "running":
            if emulator:
                emu_sensor("acceleration", 0, 0, G)
                emu_sensor("magnetic-field", 0, 20, -40)
            ask_person("Telefon flach halten und langsam drehen", watch=("heading",), label="the heading")
            s = wait(lambda s: 0 <= float(s["heading"]) < 360, "a heading", 25)
            ok(s is not None, f"heading between 0 and 360: {(s or {}).get('heading')}")
            if emulator and s is not None and sensor_state(s, "rot") != "running":
                emu_sensor("magnetic-field", 20, 0, -40)
                s = wait(lambda s: angle_diff(float(s["heading"]), 270) < 5, "heading west")
                ok(s is not None, f"north to the right of the top: heading about 270: {(s or {}).get('heading')}")
        else:
            skip("no magnetometer and no rotation estimate: no heading to check")

        # ------------------------------------------------------------ the phone's position
        print("Handylage:", flush=True)
        s = start("attitude")
        s = wait(lambda s: sensor_state(s, "accel") in ("running", "unavailable") and sensor_state(s, "gyro") in ("running", "unavailable"), "the sensors answer")
        ok(s is not None, "all five sensors have an answer: " + " ".join(f"{n}={sensor_state(s or {}, n)}" for n in ("accel", "gravity", "gyro", "magnet", "rot")))
        if emulator:
            emu_sensor("gyroscope", 0.1, 0.2, 0.3)
            s2 = wait(lambda s: sensor_state(s, "gyro") != "running" or vec(s["gyro"])[0] > 0.05, "the gyroscope's value")
            ok(s2 is not None, f"the gyroscope: {(s2 or {}).get('gyro')}")
            emu_sensor("gyroscope", 0, 0, 0)
        ask_person("Telefon in die Hand nehmen und kippen", 8, watch=("pitch", "roll"), label="pitch and roll")
        s = wait(lambda s: -90 <= float(s["pitch"]) <= 90 and -180 <= float(s["roll"]) <= 180, "pitch and roll in range")
        ok(s is not None or sensor_state(state(), "accel") == "unavailable", f"pitch, roll in range: {(s or {}).get('pitch')} {(s or {}).get('roll')}")

        # ------------------------------------------------------------ the location
        print("GPS:", flush=True)
        if sdk < 23:
            skip("no runtime permissions")
        d.sh("am", "force-stop", PKG)
        perm_set(FINE, False)
        perm_set(COARSE, False)
        if not location_enabled():
            location_set(True)
        s = start("gps")
        ok(s.get("loc.state") == "off" and s.get("loc.on") == "0", "nothing runs before the switch")
        ok(perm_state(FINE) is not None and perm_state(FINE)[0] is False, "the permission is not there yet")
        # refused
        ok(tap_label("Standort verwenden"), 'the switch "Standort verwenden"')
        if ok(dialog(DENY, "the permission dialog: deny"), "the system's dialog: refused"):
            s = wait(lambda s: s["loc.state"] == "denied", "denied")
            ok(s is not None, "refused -> denied")
        # allowed, from the switch again
        ok(tap_label("Standort verwenden"), "the switch off")
        s = wait(lambda s: s["loc.on"] == "0" and s["loc.state"] == "off", "off")
        ok(s is not None, "the switch off: off")
        ok(tap_label("Standort verwenden"), "the switch on again")
        if ok(dialog(ALLOW, "the permission dialog: allow"), "the system's dialog: allowed while in use"):
            s = wait(lambda s: s["loc.state"] in ("searching", "fix"), "searching")
            ok(s is not None, f"allowed -> {(s or {}).get('loc.state')}")
        ok(perm_state(FINE) is not None and perm_state(FINE)[0], "ACCESS_FINE_LOCATION granted")
        # a fix
        if emulator:
            d.run("emu", "geo", "fix", "11.576124", "48.137154", "519", check=False)
            s = wait(lambda s: abs(float(s["lat"]) - 48.137154) < 0.001 and abs(float(s["lon"]) - 11.576124) < 0.001, "a fix from emu geo fix", 30)
            if ok(s is not None, f"emu geo fix: latitude, longitude {(s or {}).get('lat')} {(s or {}).get('lon')}"):
                ok(s["loc.state"] == "fix", "state fix")
                ok(abs(float(s.get("altitude", "519")) - 519) < 2, f"altitude {s.get('altitude')}")
                ok(0 <= int(s["age"]) < 60000, f"age {s['age']} ms, source {s.get('source')}")
                # a second fix moves it
                d.run("emu", "geo", "fix", "13.405", "52.52", "34", check=False)
                ok(wait(lambda s: abs(float(s["lat"]) - 52.52) < 0.001, "the second fix", 30) is not None, "a second fix follows")
        else:
            ask_person("Telefon an ein Fenster oder nach draußen halten", 5)
            print(f"    (bis zu {FIX_TIMEOUT} s auf eine Position warten)", flush=True)
            s = wait(lambda s: "lat" in s, "a fix", FIX_TIMEOUT)
            if s is None:
                print("  note no fix (indoors?): not a failure", flush=True)
                notes.append("no fix (indoors?)")
            else:
                ok(-90 <= float(s["lat"]) <= 90 and -180 <= float(s["lon"]) <= 180, f"a fix: {s['lat']} {s['lon']}, accuracy {s.get('accuracy')} m, age {s['age']} ms, source {s.get('source')}")
                ok(float(s.get("accuracy", "0")) >= 0 and int(s["age"]) >= 0, "accuracy and age make sense")
                if "gps" not in s.get("source", "").lower():
                    skip(f"the fix came from {s.get('source')}: the satellites themselves not checked")
        # the system's switch
        if sdk >= 30:
            location_set(False)
            s = wait(lambda s: s["loc.state"] == "disabled", "the system's location off")
            ok(s is not None, "the system's location off -> disabled")
            location_set(True)
            s = wait(lambda s: s["loc.state"] in ("searching", "fix"), "the system's location on")
            ok(s is not None, f"on again -> {(s or {}).get('loc.state')}")
        else:
            skip("the system's location switch: API 30+")
        # the background
        go_home()
        s = wait(lambda s: s.get("suspend") == "2" and s["loc.state"] == "paused", "the location paused")
        ok(s is not None, "Home: the location paused")
        launch()
        s = wait(lambda s: s.get("suspend") == "0" and s["loc.state"] in ("searching", "fix"), "the location back")
        ok(s is not None, f"back: {(s or {}).get('loc.state')}, nobody asked")
        # refused for good: no dialog, and the process lives
        d.sh("am", "force-stop", PKG)
        perm_set(FINE, False, ["user-fixed"])
        perm_set(COARSE, False, ["user-fixed"])
        s = start("gps")
        ok(tap_label("Standort verwenden"), "the switch once more")
        s = wait(lambda s: s["loc.state"] == "denied", "denied at once", 10)
        ok(s is not None, "refused for good (user-fixed): denied, without a dialog")

        # ------------------------------------------------------------ touch: back, long press
        print("Berührung:", flush=True)
        d.sh("am", "force-stop", PKG)
        s = start("level")
        d.sh("input", "keyevent", "KEYCODE_BACK")
        s = wait(lambda s: s.get("page") == "Home" and sensor_state(s, "accel") == "off", "back on a page")
        ok(s is not None and foreground(), "the back key on a page: the start page; its sensors stop")
        d.sh("input", "keyevent", "KEYCODE_BACK")
        end = time.time() + 10
        while time.time() < end and foreground():
            time.sleep(0.3)
        ok(not foreground(), "the back key on the start page: the app goes")
        d.sh("am", "force-stop", PKG)
        launch(["--es", "jfgui_LANG", "de", "--es", "jfgui_SELFTEST", "sensors", "--es", "jfgui_PAGE", "menus"])
        n = find(label("Hier lange drücken"), '"Hier lange drücken"', 30)
        if ok(n is not None, "the page with the context menu"):
            x, y = center(n)
            d.sh("input", "swipe", str(x), str(y), str(x), str(y), "900")
            ok(find(label("Eigenschaften"), "the context menu", 10) is not None, "a long press opens the context menu")
        # a vertical swipe scrolls, but drags on a row's handle (ui.touch_drag; more in android_touch.py)
        d.sh("am", "force-stop", PKG)
        launch(["--es", "jfgui_LANG", "de", "--es", "jfgui_SELFTEST", "sensors", "--es", "jfgui_PAGE", "lists"])
        row1, row3 = find(label("Ebene 1"), '"Ebene 1"', 30), find(label("Ebene 3"), '"Ebene 3"', 10)
        if ok(row1 is not None and row3 is not None, "the page with the rows"):
            density = int(re.search(r"(\d+)\s*$", d.sh("wm", "density").strip()).group(1)) / 160
            order = lambda n: (n.get("text") or "").startswith("Reihenfolge:")
            x, y1 = center(row1)
            y3 = center(row3)[1]
            d.sh("input", "swipe", str(x), str(y3), str(x), str(y1), "500")
            time.sleep(1)
            n = find(order, "the order", 5)
            ok(n is not None and n.get("text").startswith("Reihenfolge: 1 2 3 "), "a swipe on a row's text rolls the list, the order stays")
            d.sh("input", "swipe", str(x), str(y1), str(x), str(y3), "500")  # back
            time.sleep(1)
            row1, row3 = find(label("Ebene 1"), '"Ebene 1"', 10), find(label("Ebene 3"), '"Ebene 3"', 10)
            hx = int(re.findall(r"\d+", row1.get("bounds"))[0]) - int(54 * density)
            y1, y3 = center(row1)[1], center(row3)[1]
            d.sh("input", "swipe", str(hx), str(y1), str(hx), str(y3), "900")
            time.sleep(1)
            # (how far: the last frame before the finger left; a fast swipe may end a row short)
            n = find(lambda x: (x.get("text") or "").startswith("Reihenfolge: 2 "), "the new order", 8)
            ok(n is not None, "a swipe on the handle moves the row")
    except KeyboardInterrupt:
        failures.append("interrupted")
    finally:
        restore()
        now = {p: perm_state(p) for p in (FINE, COARSE)}
        ok(all(perm_seen(now[p]) == perm_seen(saved_perms[p]) for p in now), f"the permissions as they were: {now}")
        ok(location_enabled() == saved_location, f"the system's location as it was: {saved_location}")

    print(f"android_sensors: {checks} checks passed, {len(failures)} failed" + (f"; notes: {'; '.join(notes)}" if notes else ""))
    for f in failures:
        print(f"  FAILED: {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
