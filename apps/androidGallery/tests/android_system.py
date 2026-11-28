"""Android: the system for plat on a device or emulator -- settings live, notifications, the screen
kept on, the lifecycle (android/GallerySystem.java, GalleryNotify.java, apps/androidGallery/system.rs;
lib/plat/web_system.rs, web_notify.rs, web_power.rs).

    python apps/androidGallery/tests/android_system.py [--serial SERIAL] [--apk PATH]

Starts the gallery with jfgui_SELFTEST=system (its card "06 System" at the top, every change of
its state in data://system-state.txt, read with run-as) and works it as a user would (taps found in
`uiautomator dump`), while the system changes as the user would change it:
- settings: `cmd uimode night yes|no`, `settings put system font_scale 1.3`, `settings put global
  animator_duration_scale 0`, `settings put secure high_text_contrast_enabled 1` (where the
  system lets apps read it) -> UIC_SETTINGS with the new values, the accent of API 31+;
- notifications (API 33+: the runtime permission POST_NOTIFICATIONS): not asked yet -> the
  system's dialog, allowed -> shown (`cmd notification list` / `dumpsys notification`), tapped in
  the shade -> UIC_NOTE and the activity in front; denied for good (pm revoke, user-fixed) ->
  gone at once (UIC_NOTE_GONE); granted (pm grant) and swiped away -> UIC_NOTE_GONE;
- the screen kept on: the switch -> FLAG_KEEP_SCREEN_ON (`dumpsys window`) and the partial
  WakeLock (`dumpsys power`), off again -> both gone;
- the share sheet: the button "Teilen" (plat::share: a text, a link, a file) -> SHARE_PENDING and
  the system's chooser in front; Back closes it -> SHARE_CANCELLED, the gallery in front again;
- printing (docs/ANDROID.md AN4): the button "Drucken" (plat::print: a test picture as a PDF) -> 0
  and the system's print dialog (the print spooler's PrintActivity) in front with our page (A4,
  1/1); Back closes it -> the job cancelled (logcat "print: ... -> -4"), the gallery in front;
  then "Save as PDF" through the system's save dialog -> the job done ("-> 2") and
  Download/JFLib Galerie.pdf holds the test picture (removed again; on a phone only with
  --save-pdf, an emulator always);
- the lifecycle: `input keyevent KEYCODE_HOME` -> UIC_ACTIVE 0, UIC_SUSPEND 2; started again from
  the launcher's intent -> UIC_SUSPEND 0, UIC_ACTIVE 1.
Every setting it changes is restored at the end (also after a failure or Ctrl+C): night mode,
font scale, animator scale, high-contrast text, the app's notification permission and its flags.
Runs unchanged on a phone (any serial, any ABI): without --apk it installs
build/android/androidGallery-<ro.product.cpu.abi>-debug.apk when that exists.  The phone must be
unlocked (a screen lock with a PIN cannot be dismissed from adb).  The APK must be a debug build
(jfgui_* extras and run-as).  Exit code 0 when everything passes.  adb comes from ANDROID_SDK_ROOT /
ANDROID_HOME / %LOCALAPPDATA%\\Android\\Sdk.
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
STATE = "files/fs/data/androidGallery/system-state.txt"
PERM = "android.permission.POST_NOTIFICATIONS"
DUMP = "/data/local/tmp/jfgui_system_ui.xml"
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

    def raw(self, *args, timeout=120):
        """adb's output as bytes (b"" on a timeout)"""
        try:
            return subprocess.run(self.adb + list(args), capture_output=True, timeout=timeout).stdout
        except subprocess.TimeoutExpired:
            return b""

    def prop(self, name):
        return self.sh("getprop", name).strip()


def test_picture():
    """the gallery's print picture (system.rs test_picture) as R, G, B rows"""
    w, h = 480, 320
    out = bytearray()
    for y in range(h):
        for x in range(w):
            if x < 8 or y < 8 or x >= w - 8 or y >= h - 8:
                rgb = (128, 128, 128)
            elif y < 24 or y >= h - 24 or x < 24 or x >= w - 24:
                rgb = (255, 255, 255)
            else:
                rgb = ((220, 0, 0), (0, 170, 0), (0, 0, 220), (0, 0, 0))[(x - 24) * 4 // (w - 48)]
            out += bytes(rgb)
    return bytes(out)


def saved_pdf_ok(pdf):
    """the PDF "Save as PDF" wrote (the spooler rewrites it with PDFium): an A4 page and an image
    of 480 x 320 whose Flate stream inflates to the test picture"""
    import zlib
    if not pdf.startswith(b"%PDF-"):
        return False
    box = re.search(rb"/MediaBox\s*\[\s*0\s+0\s+(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)\s*\]", pdf)
    if not box or abs(float(box.group(1)) - 595.3) > 1 or abs(float(box.group(2)) - 841.9) > 1:
        return False
    for m in re.finditer(rb"<<([^>]*?/Subtype\s*/Image[^>]*?)>>\s*stream\r?\n", pdf):
        head = m.group(1)
        w, h, n = (re.search(rb"/" + k + rb"\s+(\d+)", head) for k in (b"Width", b"Height", b"Length"))
        if not (w and h and n) or (int(w.group(1)), int(h.group(1))) != (480, 320):
            continue
        try:
            return zlib.decompress(pdf[m.end():m.end() + int(n.group(1))]) == test_picture()
        except zlib.error:
            return False
    return False


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--serial", help="adb device (default: the only one)")
    ap.add_argument("--apk", help="install this APK first (default: build/android/androidGallery-<abi>-debug.apk if it exists)")
    ap.add_argument("--save-pdf", action="store_true",
                    help="on a phone too: print to \"Save as PDF\" into its Download folder (removed again)")
    a = ap.parse_args()
    d = Device(a.serial)
    sdk = int(d.prop("ro.build.version.sdk") or "0")
    abi = d.prop("ro.product.cpu.abi")
    print(f"device: {d.prop('ro.product.model')} (API {sdk}, {abi})", flush=True)
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

    # ---------------------------------------------------------------- what is restored
    saved = {
        "night": (re.search(r"Night mode:\s*(\w+)", d.sh("cmd", "uimode", "night")) or [None, None])[1],
        "font_scale": ("system", "font_scale", d.sh("settings", "get", "system", "font_scale").strip()),
        "animator": ("global", "animator_duration_scale", d.sh("settings", "get", "global", "animator_duration_scale").strip()),
        "contrast": ("secure", "high_text_contrast_enabled", d.sh("settings", "get", "secure", "high_text_contrast_enabled").strip()),
    }
    perm_line = re.search(re.escape(PERM) + r": granted=(\w+), flags=\[([^\]]*)\]", d.sh("dumpsys", "package", PKG))
    saved_perm = (perm_line.group(1) == "true", perm_line.group(2)) if perm_line else None

    # a permission dialog left open (a failed run) stays on top of the app's task, even after a
    # force-stop: Back closes it
    def no_dialog():
        for _ in range(3):
            focus = "".join(l for l in d.sh("dumpsys", "window").splitlines() if "mCurrentFocus" in l)
            if "GrantPermissionsActivity" not in focus:
                return
            d.sh("input", "keyevent", "KEYCODE_BACK")
            time.sleep(1)

    def put(ns, key, value):
        if value in ("", "null"):
            d.sh("settings", "delete", ns, key)
        else:
            d.sh("settings", "put", ns, key, value)

    def restore():
        no_dialog()
        if saved["night"] in ("yes", "no", "auto", "custom"):
            d.sh("cmd", "uimode", "night", saved["night"])
        for ns, key, value in (saved["font_scale"], saved["animator"], saved["contrast"]):
            put(ns, key, value)
        if sdk >= 33 and saved_perm is not None:
            granted, flags = saved_perm
            d.sh("pm", "grant" if granted else "revoke", PKG, PERM)
            d.sh("pm", "clear-permission-flags", PKG, PERM, "user-set", "user-fixed")
            set_flags = [f.lower().replace("_", "-") for f in ("USER_SET", "USER_FIXED") if f in flags]
            if set_flags:
                d.sh("pm", "set-permission-flags", PKG, PERM, *set_flags)
        d.sh("cmd", "statusbar", "collapse")
        d.sh("am", "force-stop", PKG)  # its notifications go with it
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
            except (KeyError, ValueError):
                pass
            time.sleep(0.3)
        print(f"    (state: {' '.join(f'{k}={v}' for k, v in s.items())})")
        return None

    def num(s, k):
        return int(s.get(k, "0"), 0)

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
        awake()  # a long run may outlast the screen's timeout
        n = find(label(text), f'"{text}"')
        if n is None:
            return False
        tap(n)
        return True

    def posted(note_id):
        listed = d.sh("cmd", "notification", "list")
        if any(line.count("|") >= 3 for line in listed.splitlines()):  # older systems: no "list"
            return any(f"|{PKG}|{note_id}|jfgui-note|" in line for line in listed.splitlines())
        return f"Benachrichtigung Nummer {note_id}" in d.sh("dumpsys", "notification", "--noredact")

    # the system lists a note a moment after the app's call returned (seen: up to 0.4 s on an emulator)
    def listed(note_id, timeout=5):
        end = time.time() + timeout
        while time.time() < end:
            if posted(note_id):
                return True
            time.sleep(0.2)
        return False

    def focused():
        return ACTIVITY.replace("/.", "/" + PKG + ".") in "".join(
            line for line in d.sh("dumpsys", "window").splitlines() if "mCurrentFocus" in line or "mFocusedApp" in line)

    def window_flags():
        out = d.sh("dumpsys", "window", "windows")
        for block in out.split("Window #"):
            if f"{PKG}/{PKG}.GalleryActivity" in block.split("\n", 1)[0]:
                return block
        return ""

    # FLAG_KEEP_SCREEN_ON on the activity's window: AOSP prints the flags' names, One UI their
    # number; None when the window is not listed
    def keeps_screen_on():
        block = window_flags()
        if not block:
            return None
        if "KEEP_SCREEN_ON" in block:
            return True
        m = re.search(r"\bfl=(?:0x)?([0-9a-fA-F]+)\b", block)
        return m is not None and int(m.group(1), 16) & 0x80 != 0

    # the wake locks held now (dumpsys power: the "Wake Locks: size=N" block; its history below
    # names released ones too)
    def wake_locks():
        held, inside = [], False
        for line in d.sh("dumpsys", "power").splitlines():
            if line.startswith("Wake Locks:"):
                inside = True
            elif inside and line.startswith("  "):
                held.append(line.strip())
            elif inside:
                break
        return "\n".join(held)

    def launch(extra):
        d.sh("am", "start", "-W", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER",
             "-n", ACTIVITY, *extra, timeout=60)

    def start():
        awake()
        no_dialog()
        d.sh("am", "force-stop", PKG)
        d.run("shell", "run-as", PKG, "rm", "-f", STATE, check=False)
        launch(["--es", "jfgui_LANG", "de", "--es", "jfgui_SELFTEST", "system"])
        s = wait(lambda s: s.get("selftest") == "system" and s.get("active") == "1", "the app runs", 30)
        if not ok(s is not None, "the gallery runs with jfgui_SELFTEST=system and has the keyboard (UIC_ACTIVE 1)"):
            raise RuntimeError("the app did not start (a debug APK?)")
        return s

    # a button of the system's permission dialog: tapped (again while the dialog is still
    # there: a tap during its opening animation can get lost) -> whether it was there
    BUTTONS = {"permission_allow_button": ("allow", "zulassen", "erlauben"),
               "permission_deny_button": ("don't allow", "don’t allow", "nicht zulassen", "nicht erlauben")}

    def dialog(button, what):
        # the permission controller's id; its label where a system names the button otherwise
        is_button = lambda x: x.get("resource-id", "").endswith(button) or (
            x.get("package", "").endswith("permissioncontroller") and (x.get("text") or "").strip().lower() in BUTTONS[button])
        n = find(is_button, what)
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
        if sdk >= 33:
            # not asked yet (before the start: revoking a runtime permission ends the app's process)
            d.sh("am", "force-stop", PKG)
            d.sh("pm", "revoke", PKG, PERM)
            d.sh("pm", "clear-permission-flags", PKG, PERM, "user-set", "user-fixed")
        s = start()
        base = num(s, "settings")
        print(f"  start: dark_system={s['dark_system']} scale={s['scale']} motion={s['motion']} contrast={s['contrast']} accent={s['accent']}")
        if sdk >= 31:
            ok(re.fullmatch(r"[0-9A-F]{6}", s.get("accent", "")) is not None, f"the system's accent (dynamic colours): #{s.get('accent')}")

        # ------------------------------------------------------------ settings live
        print("settings:", flush=True)
        # the first step has to change something: the phone may already be dark
        modes = (("yes", "1"), ("no", "0"))
        for mode, want in (modes[::-1] if s.get("dark_system") == "1" else modes):
            d.sh("cmd", "uimode", "night", mode)
            s = wait(lambda s, w=want: s["dark_system"] == w and num(s, "settings") > base, f"night {mode}")
            if ok(s is not None and num(s, "bits") & 1, f"cmd uimode night {mode}: dark={want}, UIC_SETTINGS with SET_THEME"):
                base = num(s, "settings")
        before = state().get("scale", "100")
        d.sh("settings", "put", "system", "font_scale", "1.3")
        s = wait(lambda s: s["scale"] == "130", "font scale 1.3")
        if ok(s is not None and num(s, "bits") & 8, "font_scale 1.3: text 130 %, UIC_SETTINGS with SET_FONTS"):
            base = num(s, "settings")
        put(*saved["font_scale"])
        ok(wait(lambda s: s["scale"] == before, "font scale back") is not None, f"font scale back: text {before} %")
        d.sh("settings", "put", "global", "animator_duration_scale", "0")
        s = wait(lambda s: s["motion"] == "1", "animator scale 0")
        ok(s is not None and num(s, "bits") & 16, "animator_duration_scale 0: fewer animations, UIC_SETTINGS with SET_OTHER")
        d.sh("settings", "put", "global", "animator_duration_scale", "1")
        ok(wait(lambda s: s["motion"] == "0", "animator scale 1") is not None, "animator_duration_scale 1: animations on")
        d.sh("settings", "put", "secure", "high_text_contrast_enabled", "1")
        s = wait(lambda s: s["contrast"] == "1", "high-contrast text", 6)
        if s is not None:
            ok(num(s, "bits") & 4, "high_text_contrast_enabled 1: high contrast, UIC_SETTINGS with SET_CONTRAST")
            d.sh("settings", "put", "secure", "high_text_contrast_enabled", "0")
            ok(wait(lambda s: s["contrast"] == "0", "high-contrast text off") is not None, "high-contrast text off: normal contrast")
        else:
            notes.append("high-contrast text is not readable for apps on this system (Settings.Secure): not checked")
            print("  skip high-contrast text: not readable for apps here")
        put(*saved["animator"])
        put(*saved["contrast"])

        # ------------------------------------------------------------ notifications
        print("notifications:", flush=True)
        awake()
        n = 0
        if sdk >= 33:
            # the user refuses in the system's dialog: gone at once
            ok(tap_label("Benachrichtigung"), "tap \"Benachrichtigung\" (not asked yet)")
            n += 1
            ok(dialog("permission_deny_button", "the permission dialog"), "the system asks for POST_NOTIFICATIONS: \"Don't allow\"")
            s = wait(lambda s: num(s, "gone") == n, "refused in the dialog")
            ok(s is not None and not posted(n), f"refused: notification {n} gone (UIC_NOTE_GONE), nothing posted")
            # asked once more (Android asks twice): allowed, the waiting note is shown
            awake()
            ok(tap_label("Benachrichtigung"), "tap \"Benachrichtigung\" (asked again)")
            n += 1
            ok(dialog("permission_allow_button", "the permission dialog"), "the system asks again: \"Allow\"")
            s = wait(lambda s: num(s, "shown") == n, "shown after allowing")
            ok(s is not None and listed(n), f"allowed: notification {n} posted (cmd notification list)")
        else:
            ok(tap_label("Benachrichtigung"), "tap \"Benachrichtigung\"")
            n += 1
            ok(wait(lambda s: num(s, "shown") == n, "shown") is not None and listed(n), f"notification {n} posted")
        # tap it in the shade: UIC_NOTE, the activity in front
        d.sh("cmd", "statusbar", "expand-notifications")
        note = find(label(f"Benachrichtigung Nummer {n}"), "the notification in the shade")
        if ok(note is not None, "the notification is in the shade"):
            tap(note)
            s = wait(lambda s: num(s, "clicked") == n, "clicked")
            ok(s is not None, f"tapped: UIC_NOTE {n}")
            ok(wait(lambda s: s["active"] == "1", "active again") is not None and focused(), "the gallery is in front with the keyboard")
            ok(not posted(n), "a tapped notification goes away (auto cancel)")
        d.sh("cmd", "statusbar", "collapse")
        awake()
        ok(tap_label("Benachrichtigung"), "tap \"Benachrichtigung\" (granted)")
        n += 1
        ok(wait(lambda s: num(s, "shown") == n, "shown") is not None and listed(n), f"granted: notification {n} posted")
        d.sh("cmd", "statusbar", "expand-notifications")
        note = find(label(f"Benachrichtigung Nummer {n}"), "the notification in the shade")
        if note is not None:
            # a slow drag across most of the row (a fast `input swipe` is not taken as a dismissal)
            width = int(re.findall(r"(\d+)x\d+", d.sh("wm", "size"))[-1])
            for _ in range(3):
                x, y = center(note)
                d.sh("input", "touchscreen", "swipe", str(width * 15 // 100), str(y), str(width * 97 // 100), str(y), "800")
                time.sleep(1.5)
                if not posted(n):
                    break
                note = find(label(f"Benachrichtigung Nummer {n}"), "the notification in the shade", 5)
                if note is None:
                    break
            if posted(n):
                # SystemUI let the row snap back (seen on the emulator with swiftshader: the swipe's
                # frames are dropped); nothing was dismissed, so there is nothing to report
                notes.append("SystemUI did not take the synthetic swipe as a dismissal: UIC_NOTE_GONE through the "
                             "delete intent not checked here (swipe it away by hand to see it)")
                print("  skip swiped away: the system kept the notification")
            else:
                ok(wait(lambda s: num(s, "gone") == n, "swiped away") is not None, f"swiped away: UIC_NOTE_GONE {n} (the delete intent)")
        else:
            ok(False, "the granted notification is in the shade")
        d.sh("cmd", "statusbar", "collapse")

        # ------------------------------------------------------------ the screen kept on
        print("the screen kept on:", flush=True)
        awake()
        ok(tap_label("Bildschirm anlassen"), "tap \"Bildschirm anlassen\"")
        s = wait(lambda s: s["keep"] == "1" and num(s, "inhibited") == 3, "kept on")
        ok(s is not None, "plat::inhibit(INHIBIT_IDLE) holds idle and suspend")
        ok(keeps_screen_on() is True, "dumpsys window: the activity's window has FLAG_KEEP_SCREEN_ON")
        ok(f"jfgui:{PKG}" in wake_locks(), "dumpsys power: the partial WakeLock jfgui:org.jflib.gallery")
        ok(tap_label("Bildschirm anlassen"), "tap \"Bildschirm anlassen\" again")
        ok(wait(lambda s: s["keep"] == "0" and num(s, "inhibited") == 0, "released") is not None, "the guard dropped")
        ok(keeps_screen_on() is False, "dumpsys window: no FLAG_KEEP_SCREEN_ON")
        ok(f"jfgui:{PKG}" not in wake_locks(), "dumpsys power: the WakeLock released")

        # ------------------------------------------------------------ the share sheet
        print("the share sheet:", flush=True)
        awake()
        ok(tap_label("Teilen"), "tap \"Teilen\"")
        s = wait(lambda s: num(s, "shares") == 1 and s["share"] == "2", "share asked for")
        ok(s is not None, "plat::share: SHARE_PENDING (a text, a link and a file to the chooser)")

        def chooser_in_front():
            w = d.sh("dumpsys", "window")
            return any("mCurrentFocus" in line and ("Chooser" in line or "intentresolver" in line or "ResolverActivity" in line)
                       for line in w.splitlines())
        end = time.time() + 10
        while time.time() < end and not chooser_in_front():
            time.sleep(0.3)
        ok(chooser_in_front(), "the system's chooser is in front")
        d.sh("input", "keyevent", "KEYCODE_BACK")
        s = wait(lambda s: s["share"] == "0", "share closed")
        ok(s is not None, "closed with Back: SHARE_CANCELLED (no target chosen)")
        ok(focused(), "the gallery is in front again")

        # ------------------------------------------------------------ printing
        print("printing:", flush=True)

        def print_in_front():
            w = d.sh("dumpsys", "window")
            return any("mCurrentFocus" in line and ("printspooler" in line or "PrintActivity" in line) for line in w.splitlines())

        def spooler(rid):
            return lambda n: n.get("resource-id", "") == f"com.android.printspooler:id/{rid}"

        def print_round(n):
            """tap "Drucken" (the n-th time) -> the dialog in front with our page: A4, one page"""
            awake()
            d.sh("logcat", "-c")
            ok(tap_label("Drucken"), f"tap \"Drucken\" ({n}.)")
            s = wait(lambda s: num(s, "prints") == n, "print asked for")
            ok(s is not None and s.get("print") == "0", f"plat::print: 0, the PDF to the system's PrintManager ({s and s.get('print')})")
            end = time.time() + 20
            while time.time() < end and not print_in_front():
                time.sleep(0.3)
            if not ok(print_in_front(), "the system's print dialog (the print spooler) is in front"):
                return False
            paper = find(spooler("paper_size_summary"), "the paper in the dialog", timeout=15)
            page = find(spooler("page_number"), "the preview's page number", timeout=15)
            ok(paper is not None and "A4" in paper.get("text", ""), f"the dialog suggests the job's paper ({paper is not None and paper.get('text')})")
            ok(page is not None and page.get("text") == "1/1", f"the preview has our one page ({page is not None and page.get('text')})")
            return True

        def job_line():
            end, line = time.time() + 10, ""
            while time.time() < end and not line:
                log = d.sh("logcat", "-d", "-s", "jfgui")
                line = next((x for x in log.splitlines() if "print: " in x and " done, job " in x), "")
                time.sleep(0.3)
            return line.strip()

        if print_round(1):
            for _ in range(3):  # Back: a list that is open, then the dialog
                if focused():
                    break
                d.sh("input", "keyevent", "KEYCODE_BACK")
                time.sleep(1.5)
            ok(focused(), "Back closes the dialog, the gallery is in front again")
            line = job_line()
            ok(line.endswith("-> -4"), f"the job ended cancelled ({line or 'no line'})")

        # "Save as PDF" (the spooler's own printer) through the system's save dialog into
        # Download/: an emulator always, a phone only with --save-pdf (it writes into its Downloads)
        if a.save_pdf or d.prop("ro.kernel.qemu") == "1" or (a.serial or "").startswith("emulator-"):
            saved_pdf = "/sdcard/Download/JFLib Galerie.pdf"
            d.sh("rm", "-f", saved_pdf)
            if print_round(2):
                dest = find(spooler("title"), "the printer", timeout=10)
                if dest is not None and dest.get("text") != "Save as PDF":
                    spin = find(spooler("destination_spinner"), "the printer list")
                    if spin is not None:
                        tap(spin)
                        pdf_item = find(label("Save as PDF"), '"Save as PDF"')
                        if pdf_item is not None:
                            tap(pdf_item)
                button = find(spooler("print_button"), "the dialog's print button")
                ok(button is not None, f"the dialog's button \"{button is not None and button.get('content-desc')}\"")
                if button is not None:
                    tap(button)
                save = find(lambda n: (n.get("text") or "").lower() == "save" and n.get("clickable") == "true"
                            and "documentsui" in n.get("resource-id", "") + n.get("package", ""), "the save dialog's button")
                ok(save is not None, "the system's save dialog (DocumentsUI) is in front")
                if save is not None:
                    tap(save)
                end = time.time() + 15
                while time.time() < end and not focused():
                    time.sleep(0.3)
                ok(focused(), "saved: the gallery is in front again")
                line = job_line()
                ok(line.endswith("-> 2"), f"the job ended done ({line or 'no line'})")
                pdf = d.raw("exec-out", "cat", saved_pdf)
                ok(saved_pdf_ok(pdf), f"Download/JFLib Galerie.pdf: {len(pdf)} bytes, A4, our test picture (480 x 320) in it")
                d.sh("rm", "-f", saved_pdf)

        # ------------------------------------------------------------ the lifecycle
        print("the lifecycle:", flush=True)
        # (the print dialog and the save dialog covered the gallery: it was stopped and started before)
        before = state()
        bg, fg = num(before, "background"), num(before, "foreground")
        d.sh("input", "keyevent", "KEYCODE_HOME")
        s = wait(lambda s: s["suspend"] == "2" and s["active"] == "0" and num(s, "background") == bg + 1, "home")
        ok(s is not None, "KEYCODE_HOME: UIC_ACTIVE 0 (onPause), UIC_SUSPEND 2 (onStop)")
        launch([])
        s = wait(lambda s: s["suspend"] == "0" and s["active"] == "1" and num(s, "foreground") == fg + 1, "relaunched")
        ok(s is not None, "started again: UIC_SUSPEND 0 (onStart), UIC_ACTIVE 1 (onResume)")
        ok(focused(), "the same activity is in front")

        if sdk >= 33:
            # denied for good (Settings -> notifications off: revoked, user-fixed): no dialog,
            # gone at once; revoking ends the process, so the app starts anew
            print("notifications denied for good:", flush=True)
            d.sh("pm", "revoke", PKG, PERM)
            d.sh("pm", "set-permission-flags", PKG, PERM, "user-set", "user-fixed")
            start()
            ok(tap_label("Benachrichtigung"), "tap \"Benachrichtigung\" (denied)")
            s = wait(lambda s: num(s, "gone") == 1, "refused without a dialog")
            time.sleep(1)
            dialog_up = any(x.get("resource-id", "").endswith("permission_allow_button") for x in nodes())
            ok(s is not None and not dialog_up and not posted(1), "denied for good: no dialog, notification 1 gone at once, nothing posted")
    except (RuntimeError, KeyboardInterrupt) as e:
        failures.append(str(e) or type(e).__name__)
        print(f"stopped: {e!r}")
    finally:
        restore()
        after = [d.sh("settings", "get", ns, key).strip() for ns, key, _ in (saved["font_scale"], saved["animator"], saved["contrast"])]
        same = after == [v for _, _, v in (saved["font_scale"], saved["animator"], saved["contrast"])]
        night = (re.search(r"Night mode:\s*(\w+)", d.sh("cmd", "uimode", "night")) or [None, None])[1]
        print(f"restored: night mode {night}, font_scale {after[0]}, animator_duration_scale {after[1]}, "
              f"high_text_contrast_enabled {after[2]}" + ("" if same and night == saved["night"] else "  (DIFFERENT from before!)"))
        if not (same and night == saved["night"]):
            failures.append("settings not restored")
    for n in notes:
        print("note:", n)
    print(f"android_system: {checks} checks, " + ("ok" if not failures else f"{len(failures)} failed: " + "; ".join(failures)))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
