"""Apple packaging and starting for build_rs.py (docs/APPLE.md): a program as NAME.app, the simulator,
the iPhone.  Runs on a Mac only (Xcode's tools: codesign, xcrun simctl, xcrun devicectl).

    python tools/apple.py sim build/ios-sim/NAME [ARGS..]   start the program in a simulator (no window)
    python tools/apple.py device build/ios/NAME.app [ARGS..] sign, install and start it on the iPhone
    python tools/apple.py sign build/ios/NAME.app [--via-terminal]
                                                            sign it for the iPhone only

build_rs.py packs every program it builds for ios into NAME.app next to it (bundle); the simulator's
build is signed ad hoc there.  The iPhone needs the Apple Development certificate and a provisioning
profile that lists the device: with a free Apple ID only Xcode makes one, when a project of it ran on
the phone once (docs/APPLE.md AP0).  Such a profile is for one app id, so the app takes that id
(installs of different apps replace each other).  The keychain is locked in an SSH session, where
codesign fails with errSecInternalComponent: --via-terminal signs in the logged-in session instead
(a Terminal window opens on the Mac); `device` does that by itself when it finds itself over SSH.

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
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
IOS_MIN = "16.0"          # docs/APPLE.md, decision 7 (build_rs.py APPLE_MIN)


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
    }
    (app / "Info.plist").write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML))
    if sim:
        run(["codesign", "-f", "-s", "-", "--timestamp=none", app])
    return app


# ---------------------------------------------------------------- the simulator

def sim_device():
    """(udid, booted) of the simulator to use"""
    want = os.environ.get("jfgui_SIM_DEVICE")
    data = json.loads(run(["xcrun", "simctl", "list", "devices", "available", "-j"]).stdout)
    devices = [d for runtime, ds in data["devices"].items() if "iOS" in runtime for d in ds]
    if want:
        devices = [d for d in devices if d["udid"] == want]
    else:
        devices.sort(key=lambda d: (not d["name"].startswith("iPhone"), d["state"] != "Booted"))
    if not devices:
        raise RuntimeError("no iOS simulator available (Xcode: Settings, Components)")
    return devices[0]["udid"], devices[0]["state"] == "Booted"


def sim_run(exe, args, timeout=600):
    """start the simulator's build of a program (the plain program, not the bundle) in a simulator
    without a window: simctl spawn -> CompletedProcess"""
    udid, booted = sim_device()
    if not booted:
        run(["xcrun", "simctl", "boot", udid])
        run(["xcrun", "simctl", "bootstatus", udid, "-b"], timeout=300)
    try:
        return subprocess.run(["xcrun", "simctl", "spawn", udid, str(Path(exe).resolve()), *map(str, args)],
                              capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
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
    if len(sys.argv) < 3 or sys.argv[1] not in ("sim", "device", "sign"):
        print(__doc__)
        return 2
    cmd, path, rest = sys.argv[1], sys.argv[2], sys.argv[3:]
    try:
        if cmd == "sim":
            r = sim_run(path, rest)
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
