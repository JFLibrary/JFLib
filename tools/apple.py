"""Apple packaging and starting for build_rs.py (docs/APPLE.md): a program as NAME.app, the simulator,
the iPhone.  Runs on a Mac only (Xcode's tools: codesign, xcrun simctl, xcrun devicectl).

    python tools/apple.py sim build/ios-sim/NAME [ARGS..]   start the program in a simulator (no window)
    python tools/apple.py device build/ios/NAME.app [ARGS..] sign, install and start it on the iPhone
    python tools/apple.py sign build/ios/NAME.app [--via-terminal]
                                                            sign it for the iPhone only
    python tools/apple.py gui build/mac/NAME [ARGS..]       start a program with windows from SSH
    python tools/apple.py mac build/mac/NAME [APP]           pack a macOS program as NAME.app beside it
    python tools/apple.py dmg build/mac/NAME.app [OUT.dmg]   the bundle as a disk image (docs/APPLE.md AP8)
    python tools/apple.py notarize PATH.app|PATH.dmg         notarize and staple (Developer ID, AP8c)

build_rs.py packs every program it builds for ios into NAME.app next to it (bundle); the simulator's
build is signed ad hoc there.  The iPhone needs the Apple Development certificate and a provisioning
profile that lists the device: with a free Apple ID only Xcode makes one, when a project of it ran on
the phone once (docs/APPLE.md AP0).  Such a profile is for one app id, so the app takes that id
(installs of different apps replace each other).  The keychain is locked in an SSH session, where
codesign fails with errSecInternalComponent: --via-terminal signs in the logged-in session instead
(a Terminal window opens on the Mac); `device` does that by itself when it finds itself over SSH.

A program with windows started from SSH reaches no window server: host_exec.run_native starts the
apps with `ui` through gui_run there -- `open` starts a small runner bundle in the logged-in session,
which runs the program in the repository folder and hands back its output and exit code.  When the
program has its NAME.app beside it, the copy inside the bundle runs (its bundle identifier: macOS
gives notifications only to bundled programs).

build_rs.py packs every program it builds for mac into NAME.app too (mac_bundle, docs/APPLE.md AP7c):
Info.plist from apps/APP/PACKAGE (display, version, app_id, filetypes= as document types -- "own"
ones also exported as types of the app --, schemes= as URL types), the icon as NAME.icns from
res/app_*.png (none without), signed ad hoc; PERMISSIONS with `sandbox` turns on the App Sandbox with
the entitlements its rules need (mac_entitlements).  The signature is ad hoc unless jfgui_MAC_IDENTITY names a
certificate of the keychain ("Developer ID Application: NAME (TEAM)"): then with the hardened runtime and a
secure time stamp, as notarization wants (mac_sign).  mac_dmg puts a bundle on a compressed disk image
(hdiutil, UDZO) beside a link to /Applications; with an identity the image is signed too.  With
jfgui_MAC_NOTARY (a profile of `xcrun notarytool store-credentials`) as well, mac_dmg notarizes the
bundle and then the image and staples their tickets (mac_notarize, docs/APPLE.md AP8c).  Over SSH the
keychain is locked: codesign with a certificate and notarytool run in the logged-in session then
(keychain_run through gui_run, no window).

    jfgui_MAC_IDENTITY=NAME   sign with this certificate ("Developer ID Application: NAME (TEAM)")
    jfgui_MAC_NOTARY=PROFILE  notarize .dmg files with this notarytool profile (needs the identity)
    jfgui_SIM_DEVICE=UDID     the simulator to use (default: the first available iPhone)
    jfgui_SIM_KEEP=1          leave a simulator running that this tool booted
    jfgui_IOS_DEVICE=UDID     the iPhone (default: the only connected one)
"""
from pathlib import Path
import json
import os
import plistlib
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
IOS_MIN = "16.0"          # docs/APPLE.md, decision 7 (build_rs.py APPLE_MIN)
MAC_MIN = "13.0"
sys.path.insert(0, str(Path(__file__).resolve().parent))
import pkgmeta  # noqa: E402  (PACKAGE, PERMISSIONS, the icon)
import struct  # noqa: E402


def run(cmd, timeout=600, check=True):
    r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=timeout)
    if check and r.returncode:
        raise RuntimeError(f"{' '.join(map(str, cmd[:4]))}: {(r.stderr or r.stdout).strip()}")
    return r


def app_id(name):
    """app_id= of apps/NAME/PACKAGE, else org.jflib.NAME (the domain of the library's demos and tests)"""
    f = ROOT / "apps" / name / "PACKAGE"
    if f.exists():
        for ln in f.read_text(encoding="utf-8").splitlines():
            if ln.startswith("app_id="):
                return ln.split("=", 1)[1].strip()
    return f"org.jflib.{name}"


def ios_location(name):
    """the app may ask for the location: its PERMISSIONS name `location`, or it has none (the library's
    demos and tests show everything)"""
    f = ROOT / "apps" / name / "PERMISSIONS"
    if not f.exists():
        return (ROOT / "apps" / name).is_dir()
    return any(ln.split("#")[0].split()[:1] == ["location"] for ln in f.read_text(encoding="utf-8").splitlines())


def bundle(exe, sim=False):
    """the program exe as exe.app beside it (Info.plist, the program inside); the simulator's build
    signed ad hoc -> the bundle's path"""
    exe = Path(exe)
    app = exe.with_name(exe.name + ".app")
    app.mkdir(exist_ok=True)
    for old in app.iterdir():              # an earlier build: its program, plist and signature go
        if old.is_file():
            old.unlink()
    (app / exe.name).write_bytes(exe.read_bytes())
    (app / exe.name).chmod(0o755)
    plist = {
        "CFBundleExecutable": exe.name, "CFBundleIdentifier": app_id(exe.name), "CFBundleName": exe.name,
        "CFBundleDisplayName": exe.name, "CFBundlePackageType": "APPL", "CFBundleShortVersionString": "0.1",
        "CFBundleVersion": "1", "CFBundleSupportedPlatforms": ["iPhoneSimulator" if sim else "iPhoneOS"],
        "DTPlatformName": "iphonesimulator" if sim else "iphoneos", "MinimumOSVersion": IOS_MIN,
        "LSRequiresIPhoneOS": True, "UIDeviceFamily": [1, 2], "UILaunchScreen": {},
        # one scene, its delegate the UIKit host's (lib/plat/apple_ios.rs, docs/APPLE.md AP5)
        "UIApplicationSceneManifest": {
            "UIApplicationSupportsMultipleScenes": False,
            "UISceneConfigurations": {"UIWindowSceneSessionRoleApplication": [
                {"UISceneConfigurationName": "Default", "UISceneDelegateClassName": "JFSceneDelegate"}]},
        },
        "UISupportedInterfaceOrientations": ["UIInterfaceOrientationPortrait", "UIInterfaceOrientationLandscapeLeft",
                                             "UIInterfaceOrientationLandscapeRight"],
        "UISupportedInterfaceOrientations~ipad": ["UIInterfaceOrientationPortrait", "UIInterfaceOrientationPortraitUpsideDown",
                                                  "UIInterfaceOrientationLandscapeLeft", "UIInterfaceOrientationLandscapeRight"],
    }
    if ios_location(exe.name):          # without the text iOS never asks (plat::sensors::location_watch)
        plist["NSLocationWhenInUseUsageDescription"] = "Die App zeigt den Standort, solange er in ihr eingeschaltet ist."
    (app / "Info.plist").write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML))
    if sim:
        run(["codesign", "-f", "-s", "-", "--timestamp=none", app])
    return app


# ---------------------------------------------------------------- macOS: the app bundle

# (ICNS type, PNG size): 16, 32, 64, 128, 256 px and their @2x forms; larger ones only from a larger
# res/app_*.png (pkgmeta.icon does not scale up)
ICNS = [(b"icp4", 16), (b"icp5", 32), (b"ic11", 32), (b"icp6", 64), (b"ic12", 64), (b"ic07", 128), (b"ic13", 256),
        (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)]


def mac_icns(app):
    """the app's icon as an ICNS file (PNG entries), None without res/app_*.png"""
    res = ROOT / "apps" / app / "res"
    sizes = sorted(int(m.group(1)) for p in res.glob("app_*.png") if (m := re.fullmatch(r"app_(\d+)\.png", p.name))) \
        if res.is_dir() else []
    if not sizes:
        return None
    body = b""
    for ty, size in ICNS:
        if size > sizes[-1]:
            continue
        png = pkgmeta.icon(app, size)
        body += ty + struct.pack(">I", 8 + len(png)) + png
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def mac_entitlements(perms):
    """the App Sandbox's entitlements of a PERMISSIONS manifest -- only with `sandbox` (the opt-in of
    the OS sandbox, docs/PERMISSIONS.md); None: no sandbox.  Inside the sandbox the app's own folders
    ($CONFIG, $APPDATA, $APPCACHE, $TEMP) are its container; other fs rules become the folders macOS
    names (Downloads, Pictures, Music, Movies) or temporary exceptions (home-relative, absolute)"""
    if perms is None or not any(k == "sandbox" for k, _ in perms):
        return None
    e = {"com.apple.security.app-sandbox": True}
    folders = {"$DOWNLOADS": "files.downloads", "$PICTURES": "assets.pictures", "$MUSIC": "assets.music",
               "$VIDEOS": "assets.movies"}
    own = ("$CONFIG", "$APPDATA", "$APPCACHE", "$APPLOG", "$TEMP", "$EXE")
    home_rw, home_ro, abs_rw, abs_ro = [], [], [], []
    for key, words in perms:
        if key == "net.connect":
            e["com.apple.security.network.client"] = True
        elif key == "net.listen":
            e["com.apple.security.network.server"] = True
        elif key == "fs.user":
            e["com.apple.security.files.user-selected.read-write"] = True
        elif key == "print":
            e["com.apple.security.print"] = True
        elif key == "location":
            e["com.apple.security.personal-information.location"] = True
        elif key in ("fs.read", "fs.write", "plugins"):
            rw = key == "fs.write"
            for w in words:
                base = re.sub(r"/\*\*?$|/[^/]*\*[^/]*$", "", w)          # the folder above the glob
                top = re.match(r"\$[A-Z]+", base)
                if top and top.group(0) in own:
                    continue
                if top and top.group(0) in folders:
                    k = f"com.apple.security.{folders[top.group(0)]}.read-" + ("write" if rw else "only")
                    if not e.get(k.replace("read-only", "read-write")):
                        e[k] = True
                elif top and top.group(0) in ("$HOME", "$DOCUMENTS", "$DESKTOP"):
                    rel = base.replace("$HOME", "").replace("$DOCUMENTS", "/Documents").replace("$DESKTOP", "/Desktop")
                    (home_rw if rw else home_ro).append(rel.rstrip("/") + "/")
                elif base.startswith("/"):
                    (abs_rw if rw else abs_ro).append(base.rstrip("/") + "/")
    for k, v in (("home-relative-path.read-write", home_rw), ("home-relative-path.read-only", home_ro),
                 ("absolute-path.read-write", abs_rw), ("absolute-path.read-only", abs_ro)):
        if v:
            e["com.apple.security.temporary-exception.files." + k] = sorted(set(v))
    return e


def mac_plist(exe_name, app):
    """Info.plist of the bundle: name, id, version, icon, document and URL types from PACKAGE"""
    f = ROOT / "apps" / app / "PACKAGE"
    meta = pkgmeta.kv_file(f) if f.exists() else {}
    aid = pkgmeta.app_id(meta, app)
    version = meta.get("version", "0.1.0")
    p = {
        "CFBundleExecutable": exe_name, "CFBundleIdentifier": aid, "CFBundleName": meta.get("display", app)[:15],
        "CFBundleDisplayName": meta.get("display", app), "CFBundlePackageType": "APPL", "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleShortVersionString": version, "CFBundleVersion": version, "LSMinimumSystemVersion": MAC_MIN,
        "NSHighResolutionCapable": True, "NSPrincipalClass": "NSApplication", "CFBundleSupportedPlatforms": ["MacOSX"],
    }
    if meta.get("publisher"):
        p["NSHumanReadableCopyright"] = meta["publisher"]
    types = pkgmeta.filetypes(meta)
    if types:
        p["CFBundleDocumentTypes"] = [
            {"CFBundleTypeName": desc, "CFBundleTypeRole": "Editor", "CFBundleTypeExtensions": exts, "CFBundleTypeMIMETypes": [mime],
             "LSHandlerRank": "Owner" if own else "Alternate",
             **({"LSItemContentTypes": [f"{aid}.{exts[0]}"]} if own else {})}
            for exts, mime, desc, own in types]
        exported = [{"UTTypeIdentifier": f"{aid}.{exts[0]}", "UTTypeDescription": desc, "UTTypeConformsTo": ["public.data"],
                     "UTTypeTagSpecification": {"public.filename-extension": exts, "public.mime-type": [mime]}}
                    for exts, mime, desc, own in types if own]
        if exported:
            p["UTExportedTypeDeclarations"] = exported
    schemes = pkgmeta.schemes(meta)
    if schemes:
        p["CFBundleURLTypes"] = [{"CFBundleURLName": aid, "CFBundleURLSchemes": schemes}]
    return p


def mac_bundle(exe, app=None):
    """the macOS program exe as exe.app beside it: Contents/MacOS/NAME (a copy), Info.plist (mac_plist),
    Resources/NAME.icns, signed ad hoc (with the sandbox's entitlements when PERMISSIONS asks for it)
    -> the bundle's path"""
    exe = Path(exe)
    name, app = exe.name, app or exe.name
    bundle = exe.with_name(name + ".app")
    if bundle.exists():
        shutil.rmtree(bundle)
    contents = bundle / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    shutil.copy2(exe, contents / "MacOS" / name)
    (contents / "MacOS" / name).chmod(0o755)    # whatever copy it came from: macOS starts no program without it
    plist = mac_plist(name, app)
    icns = mac_icns(app)
    if icns:
        (contents / "Resources").mkdir()
        (contents / "Resources" / f"{name}.icns").write_bytes(icns)
        plist["CFBundleIconFile"] = name
    (contents / "Info.plist").write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML))
    (contents / "PkgInfo").write_bytes(b"APPL????")
    mac_sign(bundle, mac_entitlements(pkgmeta.permissions(app)))
    return bundle


def mac_identity():
    """the signing certificate of jfgui_MAC_IDENTITY (None: ad hoc)"""
    return os.environ.get("jfgui_MAC_IDENTITY", "").strip() or None


def mac_notary():
    """the notarytool keychain profile of jfgui_MAC_NOTARY (None: no notarization)"""
    return os.environ.get("jfgui_MAC_NOTARY", "").strip() or None


def keychain_run(cmd, timeout=600, check=True):
    """a command that needs the login keychain (codesign with a certificate, notarytool's profile).  In an
    SSH session the keychain is locked: there the command runs in the logged-in session (gui_run, no
    window; someone must be logged in) -> CompletedProcess"""
    if not os.environ.get("SSH_CONNECTION"):
        return run(cmd, timeout=timeout, check=check)
    exe = shutil.which(str(cmd[0])) or str(cmd[0])
    r = gui_run(exe, [str(c) for c in cmd[1:]], timeout=timeout)
    if check and r.returncode:
        raise RuntimeError(f"{' '.join(map(str, cmd[:4]))} (in the Mac's session): {(r.stderr or r.stdout).strip()}")
    return r


def mac_sign(path, entitlements=None):
    """sign a bundle or a disk image: ad hoc, or with jfgui_MAC_IDENTITY, the hardened runtime and a time
    stamp (what notarization requires; a disk image gets neither runtime nor entitlements)"""
    ident = mac_identity()
    path = Path(path).resolve()
    image = path.suffix == ".dmg"
    cmd = ["codesign", "-f", "-s", ident or "-"]
    cmd += ["--timestamp"] if ident else ["--timestamp=none"]
    if ident and not image:
        cmd += ["--options", "runtime"]
    with tempfile.TemporaryDirectory() as d:
        if entitlements and not image:
            ef = Path(d) / "entitlements.plist"
            ef.write_bytes(plistlib.dumps(entitlements, fmt=plistlib.FMT_XML))
            cmd += ["--entitlements", ef]
        (keychain_run if ident else run)(cmd + [path], timeout=300)


def mac_notarize(path):
    """notarize a bundle or disk image signed with the Developer ID: notarytool submits it (a bundle as a
    zip) with the keychain profile of jfgui_MAC_NOTARY and waits for Apple's verdict; the ticket is
    stapled to it, so Gatekeeper accepts it offline too.  A refusal raises with Apple's log."""
    profile, path = mac_notary(), Path(path).resolve()
    if not profile or not mac_identity():
        raise RuntimeError("notarizing needs jfgui_MAC_IDENTITY (a Developer ID) and jfgui_MAC_NOTARY (a notarytool profile)")
    with tempfile.TemporaryDirectory() as d:
        sub = path
        if path.suffix == ".app":
            sub = Path(d) / f"{path.stem}.zip"
            run(["ditto", "-c", "-k", "--keepParent", path, sub], timeout=600)
        r = keychain_run(["xcrun", "notarytool", "submit", sub, "--keychain-profile", profile, "--wait", "--timeout", "45m",
                          "--output-format", "json"], timeout=3000, check=False)
    m = re.search(r"\{.*\}", r.stdout, re.S)
    info = json.loads(m.group(0)) if m else {}
    if info.get("id") and not info.get("status"):
        raise RuntimeError(f"notarizing {path.name}: no verdict from Apple within 45 minutes (the first submissions of a new "
                           f"account can take hours); later: xcrun notarytool wait {info['id']} --keychain-profile {profile}, "
                           f"then xcrun stapler staple {path} -- or package again")
    if info.get("status") != "Accepted":
        log = ""
        if info.get("id"):
            log = keychain_run(["xcrun", "notarytool", "log", info["id"], "--keychain-profile", profile], timeout=300,
                               check=False).stdout.strip()
        raise RuntimeError(f"notarizing {path.name}: {info.get('status') or 'no verdict'} "
                           f"{(r.stderr or r.stdout).strip()[-400:]}\n{log}")
    run(["xcrun", "stapler", "staple", path], timeout=300)
    print(f"notarized: {path.name} ({info['id']})")


def mac_dmg(bundle, out=None, volume=None):
    """the bundle on a compressed disk image beside a link to /Applications (drag it there to install)
    -> the image's path (default: NAME.dmg beside the bundle).  With jfgui_MAC_IDENTITY the image is
    signed too; with jfgui_MAC_NOTARY as well the bundle (a copy dragged out works offline) and then
    the image are notarized and stapled."""
    bundle = Path(bundle)
    if bundle.suffix != ".app" or not (bundle / "Contents" / "Info.plist").exists():
        raise RuntimeError(f"{bundle} is no app bundle")
    program = bundle / "Contents" / "MacOS" / plistlib.loads((bundle / "Contents" / "Info.plist").read_bytes()).get("CFBundleExecutable", "")
    if not program.is_file() or not program.stat().st_mode & 0o111:
        raise RuntimeError(f"{program} is missing or not executable: the app would not start")
    out = Path(out) if out else bundle.with_suffix(".dmg")
    volume = volume or bundle.stem
    notarize = bool(mac_identity() and mac_notary())
    if mac_notary() and not mac_identity():
        raise RuntimeError("jfgui_MAC_NOTARY without jfgui_MAC_IDENTITY: only a Developer ID signature can be notarized")
    if notarize:
        mac_notarize(bundle)
    with tempfile.TemporaryDirectory() as d:
        stage = Path(d) / volume
        stage.mkdir()
        run(["ditto", bundle, stage / bundle.name])          # keeps the signature and the modes
        (stage / "Applications").symlink_to("/Applications")
        if out.exists():
            out.unlink()
        run(["hdiutil", "create", "-volname", volume, "-srcfolder", stage, "-fs", "HFS+", "-format", "UDZO", "-ov",
             "-quiet", out], timeout=600)
    if mac_identity():
        mac_sign(out)
    if notarize:
        mac_notarize(out)
    return out


# ---------------------------------------------------------------- macOS: windows from SSH

RUNNER = """#!/bin/sh
# jfrun DIR CODE PID PROGRAM ARGS..: PROGRAM in DIR, its pid into the file PID, its exit code into CODE
cd "$1" || exit 1
code="$2"; pidf="$3"; shift 3
"$@" &
echo $! > "$pidf"
wait $!
echo $? > "$code"
"""


def mac_runner():
    """build/mac/.runner/jfrun.app: a bundle whose program is a shell script; started through
    LaunchServices (open) it runs in the logged-in session, where windows reach the window server"""
    app = ROOT / "build" / "mac" / ".runner" / "jfrun.app"
    exe = app / "Contents" / "MacOS" / "jfrun"
    if not exe.exists() or exe.read_text(encoding="utf-8") != RUNNER:
        exe.parent.mkdir(parents=True, exist_ok=True)
        exe.write_text(RUNNER, encoding="utf-8")
        exe.chmod(0o755)
        plist = {"CFBundleExecutable": "jfrun", "CFBundleIdentifier": "org.jflib.jfrun", "CFBundleName": "jfrun",
                 "CFBundlePackageType": "APPL", "LSUIElement": True}
        (app / "Contents" / "Info.plist").write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML))
        run(["codesign", "-f", "-s", "-", app], check=False)
    return app


def gui_run(exe, args, timeout=600, env=None):
    """a program with windows from SSH: started by the runner in the logged-in session, in ROOT; the
    jfgui_* variables and those env changes are passed on -> CompletedProcess (its exit code, output).
    A timeout kills the program and raises subprocess.TimeoutExpired as subprocess.run does."""
    work = Path(tempfile.mkdtemp(prefix="jfrun-"))
    out, err, code, pidf = (work / n for n in ("out", "err", "code", "pid"))
    want = os.environ if env is None else env
    cmd = ["open", "-W", "-n", "--stdout", out, "--stderr", err]
    for k, v in want.items():
        if k.startswith("jfgui_") or (env is not None and os.environ.get(k) != v):
            cmd += ["--env", f"{k}={v}"]
    exe = Path(exe).resolve()
    inside = exe.with_name(exe.name + ".app") / "Contents" / "MacOS" / exe.name
    if inside.exists() and inside.stat().st_mtime >= exe.stat().st_mtime:
        exe = inside                       # the bundled copy: its bundle identifier counts
    cmd += [mac_runner(), "--args", ROOT, code, pidf, exe, *map(str, args)]
    try:
        subprocess.run([str(c) for c in cmd], capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.kill(int(pidf.read_text().strip()), 9)
        except (OSError, ValueError):
            pass
        shutil.rmtree(work, ignore_errors=True)
        raise
    text = code.read_text().strip() if code.exists() else ""
    rc = int(text) if text.isdigit() else 1
    result = subprocess.CompletedProcess(cmd, rc, out.read_text(encoding="utf-8", errors="replace") if out.exists() else "",
                                         err.read_text(encoding="utf-8", errors="replace") if err.exists() else "")
    shutil.rmtree(work, ignore_errors=True)
    return result


# ---------------------------------------------------------------- the simulator

def sim_device():
    """(udid, booted) of the simulator to use: a booted iPhone first; none with two displays (an
    "iPhone Duo": simctl io's screenshot wants one of them named)"""
    want = os.environ.get("jfgui_SIM_DEVICE")
    data = json.loads(run(["xcrun", "simctl", "list", "devices", "available", "-j"]).stdout)
    devices = [d for runtime, ds in data["devices"].items() if "iOS" in runtime for d in ds]
    if want:
        devices = [d for d in devices if d["udid"] == want]
    else:
        devices = [d for d in devices if "Duo" not in d["name"]]
        devices.sort(key=lambda d: (not d["name"].startswith("iPhone"), d["state"] != "Booted"))
    if not devices:
        raise RuntimeError("no iOS simulator available (Xcode: Settings, Components)")
    return devices[0]["udid"], devices[0]["state"] == "Booted"


def sim_run(exe, args, timeout=600):
    """start the simulator's build of a program (the plain program, not the bundle) in a simulator
    without a window, in the repository folder (jfgui_CWD: simctl spawn starts it in the device's data
    folder): simctl spawn -> CompletedProcess"""
    udid, booted = sim_device()
    if not booted:
        run(["xcrun", "simctl", "boot", udid])
        run(["xcrun", "simctl", "bootstatus", udid, "-b"], timeout=300)
    env = dict(os.environ, SIMCTL_CHILD_jfgui_CWD=str(ROOT))
    for k, v in os.environ.items():         # the caller's jfgui_* settings reach the program too
        if k.startswith("jfgui_"):
            env["SIMCTL_CHILD_" + k] = v
    try:
        return subprocess.run(["xcrun", "simctl", "spawn", udid, str(Path(exe).resolve()), *map(str, args)],
                              capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout,
                              env=env)
    finally:
        if not booted and os.environ.get("jfgui_SIM_KEEP") != "1":
            run(["xcrun", "simctl", "shutdown", udid], check=False)


# ---------------------------------------------------------------- the iPhone

def devicectl_json(args, timeout=120):
    with tempfile.TemporaryDirectory() as d:
        out = Path(d) / "out.json"
        run(["xcrun", "devicectl", *args, "--json-output", out], timeout=timeout)
        return json.loads(out.read_text(encoding="utf-8")).get("result", {})


def ios_device():
    """the UDID of the iPhone: jfgui_IOS_DEVICE, else the only connected physical device"""
    if os.environ.get("jfgui_IOS_DEVICE"):
        return os.environ["jfgui_IOS_DEVICE"]
    found = [d for d in devicectl_json(["list", "devices"]).get("devices", [])
             if d.get("hardwareProperties", {}).get("reality") == "physical"
             and d.get("connectionProperties", {}).get("tunnelState") != "unavailable"]
    if len(found) != 1:
        raise RuntimeError(f"{len(found)} iPhones connected: name one with jfgui_IOS_DEVICE=UDID")
    return found[0]["hardwareProperties"]["udid"]


def profile_for(udid):
    """(path, its decoded plist) of a provisioning profile that lists the device"""
    dirs = [Path.home() / "Library/Developer/Xcode/UserData/Provisioning Profiles",
            Path.home() / "Library/MobileDevice/Provisioning Profiles"]
    for d in dirs:
        for p in sorted(d.glob("*.mobileprovision"), key=lambda p: -p.stat().st_mtime) if d.is_dir() else []:
            info = plistlib.loads(run(["security", "cms", "-D", "-i", p]).stdout.encode("utf-8"))
            if udid in info.get("ProvisionedDevices", []) and info.get("ExpirationDate").timestamp() > time.time():
                return p, info
    raise RuntimeError("no valid provisioning profile lists this iPhone: run a project of Xcode on it once "
                       "(docs/APPLE.md AP0; a free Apple ID's profile lasts seven days)")


def sign(app, udid):
    """sign the bundle for the iPhone: the profile's app id (a free team's profile names one app),
    its entitlements, the profile inside, the Apple Development identity"""
    app = Path(app)
    prof, info = profile_for(udid)
    team = info["TeamIdentifier"][0]
    ent = dict(info["Entitlements"])
    bundle_id = ent["application-identifier"].split(".", 1)[1]
    plist_path = app / "Info.plist"
    plist = plistlib.loads(plist_path.read_bytes())
    if "*" in bundle_id:                      # a wildcard profile (paid team): the app keeps its own id
        bundle_id = plist["CFBundleIdentifier"]
    ent["application-identifier"] = f"{team}.{bundle_id}"
    plist["CFBundleIdentifier"] = bundle_id
    plist["CFBundleSupportedPlatforms"] = ["iPhoneOS"]
    plist_path.write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML))
    (app / "embedded.mobileprovision").write_bytes(prof.read_bytes())
    ids = run(["security", "find-identity", "-v", "-p", "codesigning"]).stdout
    m = re.search(r"\b([0-9A-F]{40})\b \"Apple Development", ids)
    if not m:
        raise RuntimeError("no valid Apple Development certificate in the keychain (Xcode: Settings, Accounts)")
    with tempfile.NamedTemporaryFile("wb", suffix=".plist", delete=False) as f:
        f.write(plistlib.dumps(ent, fmt=plistlib.FMT_XML))
    try:
        r = run(["codesign", "-f", "-s", m.group(1), "--entitlements", f.name, "--timestamp=none", app], check=False)
    finally:
        os.unlink(f.name)
    if r.returncode:
        hint = (" -- the keychain is locked in this SSH session: sign with --via-terminal (the Mac's own session)"
                if "errSecInternalComponent" in r.stderr else "")
        raise RuntimeError(f"codesign failed: {r.stderr.strip()}{hint}")
    return bundle_id


def sign_via_terminal(app, timeout=180):
    """sign in the logged-in session: a .command that runs `sign` opens in Terminal (its window stays
    on the Mac), its log comes back -> the app id"""
    app = Path(app).resolve()
    work = Path(tempfile.mkdtemp(prefix="jfgui-sign-"))
    log = work / "sign.log"
    cmd = work / "sign.command"
    cmd.write_text("#!/bin/bash\n"
                   f"cd {shlex.quote(str(ROOT))}\n"
                   f"{shlex.quote(sys.executable)} tools/apple.py sign {shlex.quote(str(app))} > {shlex.quote(str(log))} 2>&1\n"
                   f"echo \"exit $?\" >> {shlex.quote(str(log))}\n", encoding="utf-8")
    cmd.chmod(0o755)
    run(["open", cmd])
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        text = log.read_text(encoding="utf-8") if log.exists() else ""
        m = re.search(r"^exit (\d+)$", text, re.M)
        if m:
            if m.group(1) != "0":
                raise RuntimeError(f"signing in the Mac's session failed:\n{text.strip()}")
            return re.search(r"^signed as (\S+)$", text, re.M).group(1)
        time.sleep(1)
    raise RuntimeError(f"signing in the Mac's session did not finish within {timeout} s (is someone logged in?)")


def device_run(app, args, timeout=600):
    """sign (in the Mac's session when over SSH), install and start the bundle on the iPhone; its
    output -> (exit code of devicectl, output)"""
    udid = ios_device()
    bundle_id = sign_via_terminal(app) if os.environ.get("SSH_CONNECTION") else sign(app, udid)
    run(["xcrun", "devicectl", "device", "install", "app", "--device", udid, app], timeout=300)
    if devicectl_json(["device", "info", "lockState", "--device", udid]).get("passcodeRequired"):
        raise RuntimeError("the iPhone is locked: unlock it, iOS starts no app before (installed)")
    r = subprocess.run(["xcrun", "devicectl", "device", "process", "launch", "--console", "--terminate-existing",
                        "--device", udid, bundle_id, *map(str, args)],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    return r.returncode, r.stdout + r.stderr


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("sim", "device", "sign", "gui", "mac", "dmg", "notarize"):
        print(__doc__)
        return 2
    cmd, path, rest = sys.argv[1], sys.argv[2], sys.argv[3:]
    try:
        if cmd == "mac":
            print(mac_bundle(path, rest[0] if rest else None))
            return 0
        if cmd == "dmg":
            print(mac_dmg(path, rest[0] if rest else None))
            return 0
        if cmd == "notarize":
            mac_notarize(path)
            return 0
        if cmd in ("sim", "gui"):
            r = sim_run(path, rest) if cmd == "sim" else gui_run(path, rest)
            sys.stdout.write(r.stdout)
            sys.stderr.write(r.stderr)
            return r.returncode
        if cmd == "sign":
            bundle_id = sign_via_terminal(path) if "--via-terminal" in rest else sign(path, ios_device())
            print(f"signed as {bundle_id}")
            return 0
        code, output = device_run(path, rest)
        sys.stdout.write(output)
        return code
    except (RuntimeError, subprocess.TimeoutExpired) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
