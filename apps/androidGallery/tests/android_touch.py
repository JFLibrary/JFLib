"""Android: the finger (lib/ui/touch.rs, android/GalleryActivity.java) on a device or emulator.

    python apps/androidGallery/tests/android_touch.py [--serial SERIAL] [--apk PATH]

Starts the gallery on its pages and works it with `input motionevent` and `input swipe`, positions
read from `uiautomator dump`:
- a tap clicks; a finger that moves and rests moves the page by its way (1:1, also in real pixels
  of the display, whatever the library's render scale);
- a quick swipe rolls on after the finger lifted (more than its way), a touch stops that;
- a horizontal swipe rolls the row on the page "Container", a horizontal drag moves a slider, a
  vertical swipe that starts on the slider rolls the page and leaves the value alone;
- no scroll bars: the right margin of the start page stays one colour (`screencap`);
- a tool window follows the finger, stays in reach, its close button works by finger;
- the system's keyboard comes with a tap on a text field and not with a scroll.
Changes nothing on the device but the app's own state; the app is stopped at the end.  The phone must
be unlocked.  The APK must be a debug build (jfgui_* extras); without --apk it installs
build/android/androidGallery-<ro.product.cpu.abi>-debug.apk when that exists.  Exit code 0 when
everything passes.  adb comes from ANDROID_SDK_ROOT / ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
"""
import argparse
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
DUMP = "/data/local/tmp/jfgui_touch_ui.xml"
ROOT = Path(__file__).resolve().parents[3]


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

    def run(self, *args, check=True, timeout=120, binary=False):
        r = subprocess.run(self.adb + list(args), capture_output=True, timeout=timeout)
        if check and r.returncode != 0:
            raise RuntimeError(f"adb {' '.join(args)}: {r.stdout!r}{r.stderr!r}")
        return r.stdout if binary else r.stdout.decode("utf-8", "replace")

    def sh(self, *args, timeout=120):
        return self.run("shell", *args, check=False, timeout=timeout)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--apk", help="install this APK first")
    a = ap.parse_args()
    d = Device(a.serial)
    abi = d.sh("getprop", "ro.product.cpu.abi").strip()
    print(f"device: {d.sh('getprop', 'ro.product.model').strip()} ({abi})", flush=True)
    apk = Path(a.apk) if a.apk else ROOT / "build" / "android" / f"androidGallery-{abi}-debug.apk"
    if a.apk or apk.is_file():
        print(d.run("install", "-r", str(apk), timeout=300).strip().splitlines()[-1], flush=True)

    failures, checks = [], 0

    def ok(cond, what):
        nonlocal checks
        print(f"  {'ok  ' if cond else 'FAIL'} {what}", flush=True)
        if cond:
            checks += 1
        else:
            failures.append(what)
        return cond

    density = int(re.search(r"(\d+)\s*$", d.sh("wm", "density").strip()).group(1)) / 160

    def dp(v):
        return int(round(v * density))

    def nodes():
        d.sh("rm", "-f", DUMP)
        d.sh("uiautomator", "dump", DUMP, timeout=60)
        xml = d.run("exec-out", "cat", DUMP, check=False)
        start = xml.find("<?xml")
        if start < 0:
            return []
        try:
            return list(ET.fromstring(xml[start:xml.rfind(">") + 1]).iter("node"))
        except ET.ParseError:
            return []

    def label(text):
        return lambda n: text in (n.get("text"), n.get("content-desc"))

    def starts(text):
        return lambda n: (n.get("text") or "").startswith(text) or (n.get("content-desc") or "").startswith(text)

    def find(match, what, timeout=20):
        end = time.time() + timeout
        while time.time() < end:
            for n in nodes():
                if match(n):
                    return n
            time.sleep(0.4)
        print(f"    (nothing on the screen for {what})", flush=True)
        return None

    def box(n):
        return list(map(int, re.findall(r"-?\d+", n.get("bounds", "[0,0][0,0]"))))

    def center(n):
        x1, y1, x2, y2 = box(n)
        return (x1 + x2) // 2, (y1 + y2) // 2

    def text_of(n):
        return n.get("text") or n.get("content-desc") or ""

    def start(page=None):
        d.sh("input", "keyevent", "KEYCODE_WAKEUP")
        d.sh("am", "force-stop", PKG)
        extra = ["--es", "jfgui_PAGE", page] if page else []
        d.sh("am", "start", "-W", "-n", ACTIVITY, *extra, timeout=60)
        time.sleep(1.5)

    def motion(action, x=None, y=None):
        d.sh("input", "motionevent", action, *([] if x is None else [str(x), str(y)]))

    def rest_swipe(x0, y0, x1, y1, steps=4):
        """a finger from (x0, y0) to (x1, y1) that rests before it lifts: no fling"""
        motion("DOWN", x0, y0)
        for i in range(1, steps + 1):
            motion("MOVE", x0 + (x1 - x0) * i // steps, y0 + (y1 - y0) * i // steps)
        time.sleep(0.3)
        motion("UP", x1, y1)
        time.sleep(0.5)

    w, h = map(int, re.search(r"(\d+)x(\d+)", d.sh("wm", "size")).groups())
    # what the app's window starts with: below the status bar, above the navigation bar
    cx = w // 2

    try:
        # ------------------------------------------------------------ tap, pan
        print("Tippen und Rollen:", flush=True)
        start()
        n = find(label("Klick mich"), '"Klick mich"', 30)
        if not ok(n is not None, "the start page"):
            raise RuntimeError("the app did not start")
        d.sh("input", "tap", *map(str, center(n)))
        ok(find(starts("Klicks: 1"), '"Klicks: 1"', 10) is not None, "a tap clicks")
        # the rows below the button: the way of the finger, in the display's pixels
        ref = find(label("Klick mich"), '"Klick mich"', 10)
        y0 = box(ref)[1]
        way = h // 5
        rest_swipe(cx, h * 7 // 10, cx, h * 7 // 10 - way)
        ref2 = find(label("Reset"), '"Reset"', 10)
        now = find(label("Klick mich"), '"Klick mich"', 5)
        moved = y0 - box(now)[1] if now is not None else -1
        ok(abs(moved - way) <= dp(2), f"a finger that rests before it lifts moved the page by its way: {moved} of {way} px")
        ok(find(starts("Klicks: 1"), '"Klicks: 1"', 3) is not None, "the swipe was no click")
        ok(ref2 is not None, "the row beside it moved along")
        rest_swipe(cx, h * 5 // 10, cx, h * 5 // 10 + way)
        now = find(label("Klick mich"), '"Klick mich"', 5)
        ok(now is not None and abs(box(now)[1] - y0) <= dp(2), "back the same way")

        # ------------------------------------------------------------ fling
        print("Weiterrollen:", flush=True)
        start()
        n = find(label("Klick mich"), '"Klick mich"', 30)
        def first_row():
            return find(label("Klick mich"), '"Klick mich"', 3)
        far = h // 4
        y0 = box(n)[1]
        # (a busy machine's emulator may send the samples late: no speed left when it lifts; again)
        for attempt in range(3):
            d.sh("input", "swipe", str(cx), str(h * 3 // 4), str(cx), str(h * 3 // 4 - far), "60")
            time.sleep(3)
            # (the page's start is out of the dump's sight by now when it rolled on far enough)
            now = first_row()
            y1 = box(now)[1] if now is not None else -100000
            if y0 - y1 > far * (attempt + 1) * 5 // 4:
                break
        ok(y0 - y1 > far * (attempt + 1) * 5 // 4, f"a quick swipe rolls on after the finger lifted: {y0 - y1 if now is not None else 'far'} px for {far * (attempt + 1)} px of finger")
        before = nodes()
        time.sleep(1)
        after = nodes()
        ok([box(x) for x in before] == [box(x) for x in after], "and comes to rest")
        # at rest the app draws nothing
        d.sh("dumpsys", "gfxinfo", PKG, "reset")
        time.sleep(3)
        frames = re.search(r"Total frames rendered: (\d+)", d.sh("dumpsys", "gfxinfo", PKG))
        ok(frames is not None and int(frames.group(1)) <= 1, f"at rest it draws nothing: {frames.group(1) if frames else '?'} frames in 3 s")
        # the finger stops it: the page keeps still from then on
        start()
        find(label("Klick mich"), '"Klick mich"', 30)
        d.sh("input", "swipe", str(cx), str(h * 3 // 4), str(cx), str(h // 4), "40")
        d.sh("input", "motionevent", "DOWN", str(w - dp(6)), str(h // 2))
        one = nodes()
        time.sleep(0.6)
        two = nodes()
        d.sh("input", "motionevent", "UP", str(w - dp(6)), str(h // 2))
        ok([box(x) for x in one] == [box(x) for x in two], "a touch stops it")

        # ------------------------------------------------------------ no bars
        print("Rollleisten:", flush=True)
        start()
        find(label("Klick mich"), '"Klick mich"', 30)
        def margin_colours():
            raw = d.run("exec-out", "screencap", binary=True, check=False)
            sw, sh_ = int.from_bytes(raw[0:4], "little"), int.from_bytes(raw[4:8], "little")
            head = len(raw) - sw * sh_ * 4
            margin = set()
            for x in (sw - dp(2), sw - dp(4), sw - dp(6), sw - dp(7)):
                for y in range(sh_ // 8, sh_ * 3 // 4, 7):
                    at = head + (y * sw + x) * 4
                    margin.add(raw[at:at + 3])
            return len(margin)
        n = margin_colours()
        ok(n == 1, f"the right margin of the page stays one colour: {n} colours")
        # a real mouse brings them back, the next finger takes them away
        d.sh("input", "mouse", "tap", str(w - dp(3)), str(h // 2))
        time.sleep(1)
        n = margin_colours()
        ok(n > 1, f"a mouse brings the bar back: {n} colours in the margin")
        d.sh("input", "tap", str(w - dp(3)), str(h // 2))
        time.sleep(1)
        n = margin_colours()
        ok(n == 1, f"a finger takes it away again: {n} colours")

        # ------------------------------------------------------------ sideways, sliders
        print("Seitwärts, Regler:", flush=True)
        start("boxes")
        n = find(label("Rollen"), '"Rollen"', 30)
        if ok(n is not None, "the page \"Container\""):
            d.sh("input", "tap", *map(str, center(n)))
            chip = find(label("Knopf 3"), '"Knopf 3"', 10)
            if ok(chip is not None, "the row of buttons"):
                x0, y = center(chip)
                b0 = box(chip)[0]
                way = w // 3
                rest_swipe(w * 3 // 4, y, w * 3 // 4 - way, y)
                chip2 = find(label("Knopf 3"), '"Knopf 3"', 3)
                if chip2 is None:
                    chip2 = find(label("Knopf 4"), '"Knopf 4"', 3)
                # the row keeps its height, the button moved left by the way
                if ok(chip2 is not None and text_of(chip2) in ("Knopf 3", "Knopf 4"), "a horizontal swipe keeps a button in sight"):
                    want = b0 - way + (dp(104) if text_of(chip2) == "Knopf 4" else 0)
                    ok(abs(box(chip2)[0] - want) <= dp(2), f"it rolls sideways by the finger's way: {box(chip2)[0]} where {want} was expected")
                ok(find(starts("Zuletzt: Knopf 0"), '"Zuletzt: Knopf 0"', 3) is not None, "and is no click")
                chip3 = find(label("Knopf 5"), '"Knopf 5"', 3)
                if chip3 is not None:
                    d.sh("input", "tap", *map(str, center(chip3)))
                    ok(find(starts("Zuletzt: Knopf 5"), '"Zuletzt: Knopf 5"', 5) is not None, "a tap on a button of the row clicks")
        start("buttons")
        # the slider lies below the fold of a small phone: roll to it
        s = None
        for _ in range(6):
            s = next((x for x in nodes() if starts("Wert: ")(x)), None)
            if s is not None and box(s)[1] < h * 3 // 5:
                break
            rest_swipe(w - dp(8), h * 3 // 4, w - dp(8), h // 4)
        if ok(s is not None, "the page with the slider"):
            value = text_of(s)
            # the slider is the node above the "Wert" text's progress bar: just above the text
            x1, y1, x2, y2 = box(s)
            sy = y1 - dp(24)
            sl = next((x for x in nodes() if x.get("class", "").endswith("SeekBar")), None)
            sy = center(sl)[1] if sl is not None else sy
            page0 = box(s)[1]
            rest_swipe(w // 3, sy, w * 2 // 3, sy)
            now = find(starts("Wert: "), '"Wert"', 5)
            ok(now is not None and text_of(now) != value, f"a horizontal drag moves the slider: {value} -> {text_of(now) if now is not None else '?'}")
            ok(now is not None and box(now)[1] == page0, "and leaves the page")
            value = text_of(now)
            # (the page may be at its end after rolling here: then the other way)
            rest_swipe(w // 2, sy, w // 2, sy - dp(100))
            now = find(starts("Wert: "), '"Wert"', 5)
            if now is not None and box(now)[1] == page0:
                rest_swipe(w // 2, sy, w // 2, sy + dp(100))
                now = find(starts("Wert: "), '"Wert"', 5)
            ok(now is not None and text_of(now) == value, "a vertical swipe on it leaves the value")
            ok(now is not None and abs(abs(page0 - box(now)[1]) - dp(100)) <= dp(3), f"and rolls the page: {abs(page0 - box(now)[1]) if now is not None else '?'} of {dp(100)} px")

        # ------------------------------------------------------------ a window by finger
        print("Fenster:", flush=True)
        start("windows")
        n = find(label("Werkzeugfenster"), '"Werkzeugfenster"', 30)
        if ok(n is not None, "the page \"Fenster & Dialoge\""):
            d.sh("input", "tap", *map(str, center(n)))
            title = find(label("Werkzeuge"), 'the title "Werkzeuge"', 10)
            if ok(title is not None, "the tool window opens"):
                bx = box(title)
                tx, ty = center(title)
                dx, dy = dp(40), dp(90)
                rest_swipe(tx, ty, tx + dx, ty + dy)
                now = find(label("Werkzeuge"), 'the title "Werkzeuge"', 5)
                moved = (box(now)[0] - bx[0], box(now)[1] - bx[1]) if now is not None else None
                ok(moved is not None and abs(moved[0] - dx) <= dp(2) and abs(moved[1] - dy) <= dp(2), f"it follows the finger: {moved} for {(dx, dy)}")
                # up and left, far: the title bar stays in reach
                tx, ty = center(now)
                rest_swipe(tx, ty, tx - dp(50), 0, steps=2)
                rest_swipe(max(1, center(find(label("Werkzeuge"), "the title", 5))[0]), center(find(label("Werkzeuge"), "the title", 5))[1], 1, 1, steps=2)
                now = find(label("Werkzeuge"), 'the title "Werkzeuge"', 5)
                ok(now is not None and box(now)[1] >= 0 and box(now)[2] > dp(30), f"it stays in reach: {box(now) if now is not None else None}")
                tx, ty = center(now)
                rest_swipe(max(tx, 1), ty, w // 2, h // 3, steps=2)
                now = find(label("Werkzeuge"), 'the title "Werkzeuge"', 5)
                if ok(now is not None, "back in the middle"):
                    # the slider inside the window still drags, the title bar's close button by finger
                    close = find(lambda x: text_of(x) in ("Close", "Schließen") and abs(center(x)[1] - center(now)[1]) < dp(24), "the close button", 5)
                    if ok(close is not None, "the close button"):
                        d.sh("input", "tap", *map(str, center(close)))
                        time.sleep(1)
                        ok(find(label("Werkzeuge"), "", 1) is None, "the close button works by finger")

        # ------------------------------------------------------------ the keyboard
        print("Tastatur:", flush=True)
        start()
        field = None
        for _ in range(8):
            field = next((x for x in nodes() if x.get("class", "").endswith("EditText") and box(x)[3] < h * 2 // 3 and box(x)[1] > h // 8), None)
            if field is not None:
                break
            rest_swipe(w - dp(8), h * 3 // 4, w - dp(8), h // 3)
        def ime():
            return "mInputShown=true" in d.sh("dumpsys", "input_method")
        if ok(field is not None, "a text field"):
            ok(not ime(), "no keyboard at first")
            rest_swipe(w - dp(8), h // 2, w - dp(8), h // 2 - dp(40))
            ok(not ime(), "a scroll brings no keyboard")
            field = next((x for x in nodes() if x.get("class", "").endswith("EditText") and box(x)[1] > h // 8), None)
            d.sh("input", "tap", *map(str, center(field)))
            time.sleep(1.5)
            ok(ime(), "a tap on the text field brings the keyboard")
            d.sh("input", "keyevent", "KEYCODE_BACK")
            time.sleep(1)
    except KeyboardInterrupt:
        failures.append("interrupted")
    finally:
        d.sh("am", "force-stop", PKG)
        d.sh("rm", "-f", DUMP)

    print(f"android_touch: {checks} checks passed, {len(failures)} failed")
    for f in failures:
        print(f"  FAILED: {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
