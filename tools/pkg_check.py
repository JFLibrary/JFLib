"""pkg_check.py -- checks of the package formats of tools/package.py (docs/PACKAGING.md, "Checks")

    python tools/pkg_check.py --quick [--runtime FILE]     (build_rs.py test runs this)
    python tools/pkg_check.py [--app template] [--out DIR] [--keep]

  --quick: nothing is built
    meta      a fixture app (file types, URL schemes, every PERMISSIONS key, characters that need
              escaping): the .desktop entry, the AppStream metainfo, the Flatpak finish-args and
              manifest, the RPM .spec, the AppxManifest.xml (fields and mappings)
    missing   every format without its tools: skipped, the tool named, the rest written
    apps      every apps/*/PACKAGE: .desktop, metainfo (validators), AppxManifest (MakeAppx pack
              with a stand-in program)
    notices   THIRD_PARTY_NOTICES.md names every file of licenses/; the shipped notices hold it,
              licenses/ and text_shape's, and per style those of the styles whose files it embeds
    elf       pkg_linux.runtime_image on a small ELF file: "AI\\x02", .upd_info found by name,
              the image still starts at the end of the ELF data, a second write in place
    validate  (Linux / WSL) desktop-file-validate, appstreamcli validate --no-net, sh -n on the
              .spec scriptlets; (Windows) MakeAppx pack of the fixture package (schema check)
    runtime   (--runtime FILE, else build/linux/appimagert when it is there) SquashFS images
              made by mksquashfs (zstd with 4 KiB blocks: fragments, several blocks, sparse and
              uncompressed blocks; lz4 without fragments; links, a hard link, 300 entries in a
              folder) behind the runtime: --appimage-extract gives the same tree, a start runs
              AppRun with APPIMAGE / APPDIR / ARGV0 / OWD and reuses the cache, a cut image fails
  without --quick also (a few minutes): python tools/package.py APP --formats all --out DIR
    appimage  ELF with "AI\\x02", the image at --appimage-offset, the AppDir (AppRun link,
              .desktop, icon, .DirIcon, metainfo, program), both validators on it, the app's
              tests/*.uis run through the AppImage
    rpm       the .spec against its tarball (%files = the tarball's files), the scriptlets; the
              .rpm (rpm -qlp) when rpmbuild was there
    flatpak   the manifest and its files, validators; the bundle when flatpak-builder ran
    msix      the package's files and manifest, MakeAppx unpack; a test certificate made for
              this run (openssl, never installed) signs a copy, signtool verify reads it (the
              root is untrusted: expected)
    aab       what an Android App Bundle would need (bundletool, aapt2, java): only reported

Work files: build/pkg_check/ (the packages: --out, default build/pkg_check/dist; removed with the
rest unless --keep).  Prints PASS / FAIL / SKIP; exit code 1 on failures.  Nothing is installed:
not the packages, not the certificate, not a Flatpak runtime.
"""
import argparse
import base64
import contextlib
import hashlib
import io
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT))
WORK = ROOT / "build" / "pkg_check"
FAILS = []
QUIET = False


def check(ok, what):
    if not ok or not QUIET:
        print(("PASS " if ok else "FAIL ") + what)
    if not ok:
        FAILS.append(what)
    return ok


def skip(what, why):
    print(f"SKIP {what}: {why}")


# ================================================================ Linux side (run in Linux / WSL)
# python3 tools/pkg_check.py --inside-linux ACTION ARGS: stdlib only, prints PASS / FAIL lines

def tree_of(root):
    """{relative path: ("dir", mode) | ("file", mode, sha256) | ("link", target)} below root"""
    out = {}
    for dirpath, dirs, files in os.walk(root):
        for name in dirs + files:
            p = os.path.join(dirpath, name)
            st = os.lstat(p)
            rel = os.path.relpath(p, root)
            if stat.S_ISLNK(st.st_mode):
                out[rel] = ("link", os.readlink(p))
            elif stat.S_ISDIR(st.st_mode):
                out[rel] = ("dir", st.st_mode & 0o7777)
            else:
                with open(p, "rb") as f:
                    out[rel] = ("file", st.st_mode & 0o7777, hashlib.sha256(f.read()).hexdigest())
    return out


def fixture_tree(src):
    """a folder with every case the SquashFS reader must handle"""
    os.makedirs(f"{src}/a/b/c")
    os.makedirs(f"{src}/empty")
    os.makedirs(f"{src}/big")
    with open(f"{src}/small.txt", "w") as f:
        f.write("hello\n")
    open(f"{src}/zero.txt", "w").close()
    with open(f"{src}/random.bin", "wb") as f:
        f.write(os.urandom(20000))                            # incompressible: stored blocks
    with open(f"{src}/zeros.bin", "wb") as f:
        f.write(b"\0" * 50000)                                # sparse blocks
    with open(f"{src}/a/b/c/text.txt", "w") as f:
        f.write("".join(f"line {i}\n" for i in range(5000)))  # several compressed blocks
    for i in range(1, 301):                                   # two directory headers
        with open(f"{src}/big/file-with-a-longer-name-{i}.txt", "w") as f:
            f.write(f"{i}\n")
    os.symlink("small.txt", f"{src}/link")
    os.symlink("/etc/hostname", f"{src}/abs-link")
    os.link(f"{src}/small.txt", f"{src}/a/hard.txt")
    with open(f"{src}/AppRun", "w") as f:
        f.write('#!/bin/sh\necho "APPIMAGE=$APPIMAGE"\necho "APPDIR=$APPDIR"\necho "ARGV0=$ARGV0"\n'
                'echo "OWD=$OWD"\necho "ARGS=$*"\n')
    os.chmod(f"{src}/AppRun", 0o755)
    os.chmod(f"{src}/a/b", 0o700)


def inside_runtime(rt):
    """the runtime against images made by mksquashfs (zstd with fragments, lz4 without)"""
    tmp = tempfile.mkdtemp(prefix="jfgui-pkgcheck-")
    try:
        src = f"{tmp}/src"
        fixture_tree(src)
        want = tree_of(src)
        runtime = open(rt, "rb").read()
        for comp, extra in (("zstd", ["-comp", "zstd", "-b", "4096"]), ("lz4", ["-comp", "lz4", "-b", "65536", "-no-fragments"])):
            img = f"{tmp}/{comp}.squashfs"
            subprocess.run(["mksquashfs", src, img, "-all-root", "-noappend", "-quiet", "-no-progress", *extra], check=True)
            app = f"{tmp}/T-{comp}.AppImage"
            with open(app, "wb") as f:
                f.write(runtime + open(img, "rb").read())
            os.chmod(app, 0o755)
            r = subprocess.run([app, "--appimage-offset"], capture_output=True, text=True)
            check(r.returncode == 0 and r.stdout.strip() == str(len(runtime)), f"runtime {comp}: --appimage-offset is the runtime's size")
            work = f"{tmp}/x-{comp}"
            os.makedirs(work)
            r = subprocess.run([app, "--appimage-extract"], cwd=work, capture_output=True, text=True)
            got = tree_of(f"{work}/squashfs-root") if r.returncode == 0 else {}
            diff = sorted(k for k in set(want) | set(got) if want.get(k) != got.get(k))
            check(r.returncode == 0 and not diff, f"runtime {comp}: --appimage-extract gives the same tree ({len(want)} entries)"
                  + (f" -- differs: {diff[:5]} {r.stderr.strip()}" if diff or r.returncode else ""))
            r = subprocess.run([app, "--appimage-extract", "big/*-2??.txt"], cwd=work, capture_output=True, text=True)
            check(r.returncode == 0 and r.stdout.startswith("100 "), f"runtime {comp}: --appimage-extract PATTERN")
            planted = f"{tmp}/planted-{comp}"                    # a link left in squashfs-root is replaced, never followed
            os.makedirs(f"{planted}/squashfs-root")
            os.makedirs(f"{tmp}/outside-{comp}")
            os.symlink(f"{tmp}/outside-{comp}", f"{planted}/squashfs-root/big")
            r = subprocess.run([app, "--appimage-extract", "big/*-2??.txt"], cwd=planted, capture_output=True, text=True)
            check(r.returncode == 0 and not os.listdir(f"{tmp}/outside-{comp}") and not os.path.islink(f"{planted}/squashfs-root/big")
                  and len(os.listdir(f"{planted}/squashfs-root/big")) == 100,
                  f"runtime {comp}: a link in squashfs-root is replaced, nothing written through it")
            env = dict(os.environ, XDG_CACHE_HOME=f"{tmp}/cache-{comp}")
            r1 = subprocess.run([app, "one", "two words"], cwd=tmp, env=env, capture_output=True, text=True)
            lines = dict(ln.split("=", 1) for ln in r1.stdout.splitlines() if "=" in ln)
            appdir = lines.get("APPDIR", "")
            check(r1.returncode == 0 and lines.get("APPIMAGE") == app and lines.get("ARGV0") == app and lines.get("OWD") == tmp
                  and lines.get("ARGS") == "one two words" and appdir.startswith(f"{tmp}/cache-{comp}/appimage-run/T-{comp}-")
                  and os.path.isfile(f"{appdir}/random.bin"),
                  f"runtime {comp}: a start extracts into the cache and runs AppRun (APPIMAGE, APPDIR, ARGV0, OWD, arguments)"
                  + ("" if r1.returncode == 0 else f" -- {r1.stderr.strip()}"))
            os.remove(f"{appdir}/random.bin")                    # the cache is used, not extracted again
            r2 = subprocess.run([app, "again"], cwd=tmp, env=env, capture_output=True, text=True)
            check(r2.returncode == 0 and f"APPDIR={appdir}" in r2.stdout and not os.path.exists(f"{appdir}/random.bin")
                  and len(os.listdir(f"{tmp}/cache-{comp}/appimage-run")) == 1, f"runtime {comp}: the next start reuses the cache")
        cut = f"{tmp}/cut.AppImage"
        data = open(f"{tmp}/T-zstd.AppImage", "rb").read()
        with open(cut, "wb") as f:
            f.write(data[:len(runtime) + (len(data) - len(runtime)) // 2])
        os.chmod(cut, 0o755)
        r = subprocess.run([cut], cwd=tmp, env=dict(os.environ, XDG_CACHE_HOME=f"{tmp}/cache-cut"), capture_output=True, text=True)
        check(r.returncode == 1 and "appimagert:" in r.stderr, f"runtime: a cut image fails with a message ({r.stderr.strip()[:80]})")
        r = subprocess.run([f"{tmp}/T-zstd.AppImage", "--appimage-mount"], capture_output=True, text=True)
        check(r.returncode == 1 and "FUSE" in r.stderr, "runtime: --appimage-mount is refused (no FUSE)")
        r = subprocess.run([f"{tmp}/T-zstd.AppImage", "--appimage-version"], capture_output=True, text=True)
        check(r.returncode == 0 and "JFLib" in r.stdout, "runtime: --appimage-version")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def inside_validate(files):
    """desktop-file-validate / appstreamcli validate on files (by their names), sh -n on scriptlets;
    "--no-homepage" before a metainfo file: its missing homepage is only an info (package.py says so)"""
    lax = False
    for f in files:
        if f == "--no-homepage":
            lax = True
            continue
        if f.endswith(".desktop"):
            if not shutil.which("desktop-file-validate"):
                print(f"SKIP desktop-file-validate: not installed")
                continue
            r = subprocess.run(["desktop-file-validate", f], capture_output=True, text=True)
            bad = [ln for ln in r.stdout.splitlines() if re.search(r": (error|warning): ", ln)]
            check(r.returncode == 0 and not bad, f"desktop-file-validate {os.path.basename(f)}"
                  + (f" -- {r.stdout.strip()} {r.stderr.strip()}" if r.returncode or bad else ""))
            for ln in r.stdout.splitlines():
                if ": hint: " in ln:                            # advice, not a fault
                    print(f"INFO desktop-file-validate {os.path.basename(f)}: {ln.split(': hint: ', 1)[1]}")
        elif f.endswith(".xml"):
            if not shutil.which("appstreamcli"):
                print(f"SKIP appstreamcli: not installed")
                continue
            r = subprocess.run(["appstreamcli", "validate", "--no-net", "--no-color", *(["--override=url-homepage-missing=info"] if lax else []), f],
                               capture_output=True, text=True)
            lax = False
            check(r.returncode == 0, f"appstreamcli validate {os.path.basename(f)}"
                  + (f" -- {r.stdout.strip()} {r.stderr.strip()}" if r.returncode else ""))
        elif f.endswith(".sh"):
            r = subprocess.run(["sh", "-n", f], capture_output=True, text=True)
            check(r.returncode == 0, f"sh -n {os.path.basename(f)}" + (f" -- {r.stderr.strip()}" if r.returncode else ""))


def inside_appimage(app, scripts):
    """a packaged AppImage: the image's listing, the AppDir, the validators, the app's scripts"""
    tmp = tempfile.mkdtemp(prefix="jfgui-pkgcheck-")
    try:
        r = subprocess.run([app, "--appimage-extract"], cwd=tmp, capture_output=True, text=True)
        root = f"{tmp}/squashfs-root"
        if not check(r.returncode == 0, "appimage: --appimage-extract" + (f" -- {r.stderr.strip()}" if r.returncode else "")):
            return
        t = tree_of(root)
        print("INFO appimage: " + " ".join(sorted(k for k, v in t.items() if v[0] != "dir" and "/doc/" not in k)))
        desk = [k for k in t if k.endswith(".desktop") and "/" not in k]
        icons = [k for k in t if k.endswith(".png") and "/" not in k]
        apprun = t.get("AppRun", ("",))
        check(len(desk) == 1 and len(icons) == 1 and t.get(".DirIcon") == ("link", icons[0])
              and apprun[0] == "link" and os.access(os.path.join(root, apprun[1]), os.X_OK),
              "appimage: AppRun (a link to the program), one .desktop and its icon at the top, .DirIcon")
        aid = desk[0][:-len(".desktop")] if desk else ""
        check(f"usr/share/metainfo/{aid}.appdata.xml" in t and f"usr/share/applications/{aid}.desktop" in t
              and f"usr/share/icons/hicolor/256x256/apps/{aid}.png" in t, "appimage: metainfo, .desktop and icons below usr/share")
        inside_validate([os.path.join(root, desk[0]), os.path.join(root, f"usr/share/metainfo/{aid}.appdata.xml")] if desk else [])
        if shutil.which("unsquashfs"):
            off = subprocess.run([app, "--appimage-offset"], capture_output=True, text=True).stdout.strip()
            r = subprocess.run(["unsquashfs", "-o", off, "-l", app], capture_output=True, text=True)
            check(r.returncode == 0 and "squashfs-root/AppRun" in r.stdout, "appimage: unsquashfs reads the image at --appimage-offset")
        env = dict(os.environ, XDG_CACHE_HOME=f"{tmp}/cache")
        for s in scripts:
            r = subprocess.run([app, "--script", s], env=env, capture_output=True, text=True, timeout=300)
            check(r.returncode == 0, f"appimage: {os.path.basename(s)} runs through the AppImage"
                  + (f" -- exit {r.returncode}: {(r.stdout + r.stderr).strip()[-400:]}" if r.returncode else ""))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def inside_flatpak(src, aid, root, scripts):
    """flatpak-builder on the written manifest (a build folder only, nothing exported or
    installed), the build's metadata, then the app's scripts in its sandbox (flatpak build)"""
    tmp = tempfile.mkdtemp(prefix="jfgui-pkgcheck-")
    try:
        shutil.copytree(src, f"{tmp}/src")
        r = subprocess.run(["flatpak-builder", "--disable-rofiles-fuse", "--force-clean", "--disable-updates",
                            f"--state-dir={tmp}/state", f"{tmp}/build", f"{tmp}/src/{aid}.json"], capture_output=True, text=True)
        if not check(r.returncode == 0, "flatpak: flatpak-builder builds the manifest" + (f" -- {r.stderr.strip()[-600:]}" if r.returncode else "")):
            return
        meta = open(f"{tmp}/build/metadata").read()
        check(f"name={aid}" in meta and "[Context]" in meta, "flatpak: the build's metadata (name, [Context] of the finish-args)")
        print("INFO flatpak metadata: " + " | ".join(ln for ln in meta.splitlines() if ln and not ln.startswith("[")))
        exported = f"{tmp}/build/export/share/applications/{aid}.desktop"
        check(os.path.exists(exported), "flatpak: the .desktop file is exported")
        cmd = json.load(open(f"{tmp}/src/{aid}.json"))["command"]
        for s in scripts:
            r = subprocess.run(["flatpak", "build", "--filesystem=host", "--share=ipc", "--socket=x11", "--socket=wayland",
                                f"{tmp}/build", cmd, "--script", s], cwd=root, capture_output=True, text=True, timeout=300)
            check(r.returncode == 0, f"flatpak: {os.path.basename(s)} runs in the sandbox (flatpak build)"
                  + (f" -- exit {r.returncode}: {(r.stdout + r.stderr).strip()[-400:]}" if r.returncode else ""))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def inside_main(args):
    action = args[0]
    if action == "runtime":
        inside_runtime(args[1])
    elif action == "validate":
        inside_validate(args[1:])
    elif action == "appimage":
        inside_appimage(args[1], args[2:])
    elif action == "flatpak":
        inside_flatpak(args[1], args[2], args[3], args[4:])
    return 1 if FAILS else 0


# ================================================================ host side

def run_inside(*args, timeout=900):
    """this file's Linux part in Linux / WSL: its PASS / FAIL lines are counted here"""
    from tools.host_exec import linux_path, run_linux
    r = run_linux(["python3", linux_path(Path(__file__)), "--inside-linux", *map(str, args)], timeout=timeout)
    for line in r.stdout.splitlines():
        if line.startswith("FAIL "):
            FAILS.append(line[5:])
            print(line)
        elif line.startswith(("PASS ", "SKIP ", "INFO ")):
            if not QUIET or line.startswith("SKIP "):
                print(line)
    if r.returncode and not any(line.startswith("FAIL ") for line in r.stdout.splitlines()):
        check(False, f"Linux part {args[0]}: exit {r.returncode} {(r.stdout + r.stderr).strip()[-600:]}")


def lp(p):
    from tools.host_exec import linux_path
    return linux_path(p)


import pkg_linux  # noqa: E402
import pkg_msix  # noqa: E402
import pkgmeta  # noqa: E402

FIX_META = {"display": 'Prüf & "Fixture"', "version": "1.2.3-beta", "publisher": "JFLib Test", "license": "MIT",
            "summary": "Prüfpaket für 100 % der Felder", "description": "Ein Paket, das jede Abbildung prüft <b>.",
            "generic": "Prüfprogramm", "categories": "Utility;", "deb_name": "jf-fixture", "app_id": "org.jflib.Fixture",
            "filetypes": "fxd|application/x-jf-fixture|Fixture-Dokument|own; png,apng|image/png|PNG-Bild",
            "schemes": "jffix", "styles": "winui kde", "default_win": "winui", "default_linux": "kde",
            "homepage": "https://example.org/fixture", "flatpak_args": "--talk-name=org.kde.StatusNotifierWatcher",
            "unit_jf-fixture-daemon": "Fixture-Dienst|jf-fixture-daemon run"}
FIX_PERMS = [("fs.read", ["$PICTURES/**", "/opt/fixture/data/*.bin"]),
             ("fs.write", ["$DOCUMENTS/Fixture/**", "$CONFIG/Fixture/**", "$TEMP/fx-*/**", "C:/Fixture/**", "$HOME/.fixture/**"]),
             ("fs.user", []), ("net.connect", ["api.example.org:443"]), ("net.listen", ["127.0.0.1:0"]), ("secrets", []),
             ("autostart", []), ("audio.output", []), ("process", ["gmic", "$EXE/helper"]), ("print", []), ("url", ["https"]),
             ("plugins", ["$EXE/plugins/*.so", "/opt/fixture/plugins/*.so"])]


def fixture_job(d, perms=FIX_PERMS):
    """a Job of the fixture app with stand-in files in d"""
    d.mkdir(parents=True, exist_ok=True)
    for name in ("Fixture-winui", "Fixture-kde", "jf-fixture-daemon", "jxlenc.so"):
        (d / name).write_bytes(b"\x7fELF" + name.encode())
    notice = d / "OFL.txt"
    notice.write_text("notice\n", encoding="utf-8")
    exe = d / "Fixture.exe"
    if not exe.exists():
        shutil.copyfile(sys.executable, exe)                  # any PE file: MakeAppx checks the manifest, not the program
    return pkgmeta.Job(app="Fixture", meta=dict(FIX_META), styles=["winui", "kde"], root=ROOT, dist=d / "dist", work=d / "work",
                       impl="rs", pkg="jf-fixture", version=FIX_META["version"], perms=perms,
                       exe={"winui": {"linux": d / "Fixture-winui", "win": exe}, "kde": {"linux": d / "Fixture-kde", "win": exe}},
                       extra={"linux": [("jxlenc.so", d / "jxlenc.so"), ("jf-fixture-daemon", d / "jf-fixture-daemon")], "win": []},
                       bins={"linux": ["jf-fixture-daemon"], "win": []}, cross={"linux": [], "win": []},
                       notices={Path("licenses/fonts/OFL.txt"): notice},
                       units=[("jf-fixture-daemon", "[Unit]\nDescription=Fixture\n\n[Service]\nExecStart=/usr/lib/jf-fixture/jf-fixture-daemon run\n")],
                       runtime=None, appimage_runtime=None)


def t_meta(d):
    """the fixture's metadata in every format -> the files for the validators"""
    saved = os.environ.pop("jfgui_MSIX_PUBLISHER", None)
    try:
        return meta_checks(d)
    finally:
        if saved is not None:
            os.environ["jfgui_MSIX_PUBLISHER"] = saved


def meta_checks(d):
    pkgmeta._warned.add("Fixture")                            # its placeholder icons need no note
    job = fixture_job(d)
    out = {}
    desk = pkgmeta.desktop_entry(job.meta, job.app, "jf-fixture", "org.jflib.Fixture", "jf-fixture")
    check("Exec=jf-fixture %U\n" in desk and "MimeType=application/x-jf-fixture;image/png;x-scheme-handler/jffix;\n" in desk
          and "StartupWMClass=jf-fixture\n" in desk and "Icon=org.jflib.Fixture\n" in desk,
          "meta: .desktop: Exec with %U (schemes), MimeType of file types and schemes, StartupWMClass, Icon")
    (d / "org.jflib.Fixture.desktop").write_bytes(pkgmeta.lf(desk))
    out["desktop"] = d / "org.jflib.Fixture.desktop"
    mi = pkgmeta.metainfo(job.meta, job.app, "org.jflib.Fixture", "jf-fixture")
    check("<id>org.jflib.Fixture</id>" in mi and "<name>Prüf &amp; \"Fixture\"</name>" in mi and "&lt;b&gt;" in mi
          and '<launchable type="desktop-id">org.jflib.Fixture.desktop</launchable>' in mi
          and "<mediatype>application/x-jf-fixture</mediatype>" in mi and '<release version="1.2.3-beta"' in mi
          and "<project_license>MIT</project_license>" in mi and '<developer id="org.jflib">' in mi,
          "meta: metainfo: id, escaped name and description, launchable, media types, release, license, developer")
    (d / "org.jflib.Fixture.metainfo.xml").write_bytes(pkgmeta.lf(mi))
    out["metainfo"] = d / "org.jflib.Fixture.metainfo.xml"
    for bad in ("Fixture", "org.jflib", "org.1jflib.App", "org.jf-lib.App"):
        try:
            pkgmeta.app_id({"app_id": bad}, "X")
            check(False, f"meta: app_id {bad!r} is refused")
        except SystemExit:
            pass
    check(pkgmeta.app_id({}, "Paint Craft") == "org.jflib.Paint_Craft", "meta: app_id default org.jflib.<App>, bad app ids refused")

    # Flatpak
    args, notes = pkg_linux.finish_args(FIX_PERMS, ["--talk-name=org.kde.StatusNotifierWatcher"])
    want = ["--share=ipc", "--socket=wayland", "--socket=fallback-x11", "--share=network", "--talk-name=org.freedesktop.secrets",
            "--socket=pulseaudio", "--filesystem=/opt/fixture/data:ro", "--filesystem=/opt/fixture/plugins:ro", "--filesystem=xdg-documents/Fixture",
            "--filesystem=xdg-pictures:ro", "--filesystem=~/.fixture", "--talk-name=org.kde.StatusNotifierWatcher"]
    check(args == want, f"flatpak: finish-args of every PERMISSIONS key ({' '.join(args)})")
    check(any("$CONFIG" in n for n in notes) and any("$TEMP" in n for n in notes) and any("gmic" in n for n in notes)
          and not any("helper" in n for n in notes) and any(n.startswith("fs.user") for n in notes),
          "flatpak: notes for the folders inside the sandbox, programs outside it, the portals")
    a2, n2 = pkg_linux.finish_args(None)
    check("--filesystem=host" in a2 and "--share=network" in a2 and "--socket=pulseaudio" in a2 and n2,
          "flatpak: no PERMISSIONS: wide holes (also the sound server) and a note")
    a3, _ = pkg_linux.finish_args([("fs.user", [])])
    check(a3 == ["--share=ipc", "--socket=wayland", "--socket=fallback-x11"], "flatpak: fs.user alone: no file system hole, no sound server")
    a4, n4 = pkg_linux.finish_args([("fs.user", []), ("audio.output", []), ("audio.output", [])])
    check(a4 == ["--share=ipc", "--socket=wayland", "--socket=fallback-x11", "--socket=pulseaudio"] and any(n.startswith("audio.output") for n in n4),
          "flatpak: audio.output: --socket=pulseaudio once, with a note")
    files = pkg_linux.flatpak_files(job)
    man, _ = pkg_linux.flatpak_manifest(job, files)
    check(man["id"] == "org.jflib.Fixture" and man["command"] == "jf-fixture" and man["runtime"] == "org.freedesktop.Platform"
          and man["sdk"] == "org.freedesktop.Sdk" and man["finish-args"] == want
          and "lib/jf-fixture/jf-fixture" in files and "share/applications/org.jflib.Fixture.desktop" in files
          and "share/metainfo/org.jflib.Fixture.metainfo.xml" in files and "share/mime/packages/org.jflib.Fixture.xml" in files
          and "ln -s ../lib/jf-fixture/jf-fixture /app/bin/jf-fixture" in man["modules"][0]["build-commands"],
          "flatpak: manifest (id, command, runtime, finish-args) and its files (the default style kde)")
    check(files["lib/jf-fixture/jf-fixture"][0] == job.exe["kde"]["linux"], "flatpak: the program is the default Linux style")

    # RPM
    tree = pkg_linux.rpm_tree(job)
    spec = pkg_linux.rpm_spec(job, "jf-fixture-1.2.3~beta-root.tar.gz", tree)
    listed = sorted(m.group(1) for m in re.finditer(r'^"/([^"]+)"$', spec, re.M))
    check(listed == sorted(tree) and "Version:        1.2.3~beta\n" in spec and "Summary:        Prüfpaket für 100 %% der Felder\n" in spec
          and "usr/lib/systemd/system/jf-fixture-daemon.service" in tree and "usr/lib/jf-fixture/Fixture-kde" in tree
          and "Fixture-kde 50" in spec and "Fixture-winui 10" in spec and "ln -sf /usr/lib/jf-fixture/jf-fixture-daemon" in spec
          and '%dir "/usr/lib/jf-fixture"' in spec and "License:        MIT\n" in spec,
          "rpm: .spec: every tree file in %files, version ~, % escaped, alternatives (default kde first), units in /usr/lib")
    (d / "rpm.spec").write_bytes(pkgmeta.lf(spec))
    out["scriptlets"] = []
    for part in ("post", "preun"):
        m = re.search(rf"^%{part}\n(.*?)(?=^%|\Z)", spec, re.M | re.S)
        body = (m.group(1) if m else "").replace("%{_sbindir}", "/usr/sbin").replace("%{_bindir}", "/usr/bin").replace("$1", "0")
        f = d / f"rpm-{part}.sh"
        f.write_bytes(pkgmeta.lf("#!/bin/sh\n" + body))
        out["scriptlets"].append(f)
    raw = pkg_linux.tar_gz(tree, 0)
    with tarfile.open(fileobj=io.BytesIO(raw)) as t:
        names = {m.name[2:]: m for m in t.getmembers()}
    check(all(n in names and names[n].uid == 0 and names[n].mode == tree[n][1] for n in tree) and names["usr/lib/jf-fixture"].isdir(),
          "rpm: the root tarball holds every file with its mode, owned by root, and the folders")

    # MSIX
    xml = pkg_msix.appx_manifest(job, "winui")
    check('Name="org.jflib.Fixture"' in xml and 'Publisher="CN=JFLib Test"' in xml and 'Version="1.2.3.0"' in xml
          and '<uap:FileTypeAssociation Name="fxd">' in xml and '<uap:FileType ContentType="image/png">.apng</uap:FileType>' in xml
          and '<uap:Protocol Name="jffix">' in xml and 'TaskId="Autostart" Enabled="false"' in xml
          and '<Capability Name="internetClient"/>' in xml and '<Capability Name="privateNetworkClientServer"/>' in xml
          and '<rescap:Capability Name="runFullTrust"/>' in xml and 'DisplayName="Prüf &amp; &quot;Fixture&quot;"' in xml,
          "msix: AppxManifest: identity, version a.b.c.0, file types, protocol, StartupTask, capabilities, escaping")
    job_none = fixture_job(d, perms=[("fs.user", [])])
    xml2 = pkg_msix.appx_manifest(job_none, "winui")
    check("StartupTask" not in xml2 and "internetClient" not in xml2, "msix: without autostart / net rules: no StartupTask, no capabilities")
    caps, cnotes = pkg_msix.capabilities([("fs.user", []), ("audio.output", [])])
    check(caps == [] and any(n.startswith("audio.output") for n in cnotes), "msix: audio.output needs no capability (noted)")
    for bad_meta, what in (({"schemes": "https"}, "a reserved scheme"), ({"version": "70000.1"}, "a version part > 65535"),
                           ({"msix_name": "x"}, "a short identity name"),
                           ({"filetypes": "fxd|application/x-jf-fixture|Fixture|own; png,fxd|image/png|PNG"}, "an extension in two file types")):
        try:
            j = fixture_job(d)
            j.meta.update(bad_meta)
            pkg_msix.appx_manifest(j, "winui")
            check(False, f"msix: {what} is refused")
        except SystemExit:
            pass
    return job, out


def t_missing(d):
    """every format without its tools: skipped with what is missing, the rest written, no error"""
    job = fixture_job(d / "missing")
    saved_probe, saved_find = pkg_linux._probe, pkg_msix.find_makeappx
    pkg_linux._probe = {t: "" for t in ("mksquashfs", "appimagetool", "rpmbuild", "flatpak", "flatpak-builder")}
    pkg_linux._probe["flatpak_refs"] = []
    pkg_msix.find_makeappx = lambda: None
    try:
        with contextlib.redirect_stdout(io.StringIO()):         # package.py's progress lines
            results = [pkg_linux.make_appimage(job, "kde"), pkg_linux.make_rpm(job), pkg_linux.make_flatpak(job),
                       pkg_msix.make_msix(job, None)]
    finally:
        pkg_linux._probe, pkg_msix.find_makeappx = saved_probe, saved_find
    why = dict(job.skipped)
    check(results == [None] * 4 and "mksquashfs" in why.get("appimage", "") and "rpmbuild" in why.get("rpm", "")
          and "flatpak-builder" in why.get("flatpak", "") and "MakeAppx" in why.get("msix", ""),
          "missing tools: each format is skipped and names what is missing")
    check((job.work / "appimage" / "kde" / "AppDir" / "org.jflib.Fixture.desktop").exists()
          and (job.dist / "rpm" / "SPECS" / "jf-fixture.spec").exists() and (job.dist / "flatpak" / "org.jflib.Fixture.json").exists()
          and (job.work / "msix" / "winui" / "AppxManifest.xml").exists() and not job.done,
          "missing tools: AppDir, .spec, Flatpak manifest and the MSIX folder are still written")


def t_elf():
    """runtime_image on a small ELF64 file"""
    names = b"\0.text\0.shstrtab\0"
    text = b"\xc3" * 16
    shoff = 64 + len(text) + len(names)
    shoff += -shoff % 8
    head = bytearray(64)
    head[:16] = b"\x7fELF\x02\x01\x01" + b"\0" * 9
    struct.pack_into("<HHIQQQIHHHHHH", head, 16, 2, 62, 1, 0, 0, shoff, 0, 64, 56, 0, 64, 3, 2)
    body = bytes(head) + text + names
    body += b"\0" * (shoff - len(body))
    sh = bytes(64)
    sh += struct.pack("<IIQQQQIIQQ", 1, 1, 6, 0, 64, len(text), 0, 0, 16, 0)
    sh += struct.pack("<IIQQQQIIQQ", 7, 3, 0, 0, 64 + len(text), len(names), 0, 0, 1, 0)
    elf = body + sh
    check(pkg_linux.elf_end(elf) == len(elf), "elf: elf_end of a plain file is its size")
    rt = pkg_linux.runtime_image(elf, "zsync|https://example.org/app.zsync")
    sec = pkg_linux.elf_sections(rt)
    off, size = sec.get(".upd_info", (0, 0))
    check(rt[8:11] == b"AI\x02" and size == 1024 and rt[off:off + 35] == b"zsync|https://example.org/app.zsync"
          and rt[off + 35] == 0 and ".text" in sec and pkg_linux.elf_end(rt) == len(rt),
          "elf: AppImage magic, .upd_info (1024 bytes) added and found by name, the image starts at the end")
    rt2 = pkg_linux.runtime_image(rt, "gh-releases-zsync|a|b|latest|x.zsync")
    off2, _ = pkg_linux.elf_sections(rt2)[".upd_info"]
    check(len(rt2) == len(rt) and rt2[off2:off2 + 9] == b"gh-releas" and rt2[off2 + 37] == 0, "elf: a second write goes into the same section")


def makeappx_pack(job, d):
    """MakeAppx pack of the job's package folder -> (ok, output)"""
    tool = pkg_msix.find_makeappx()
    if tool is None:
        return None, "MakeAppx.exe not found"
    lay = d / "msix-layout"
    pkg_msix.layout(job, job.default_style("win"), lay)
    r = subprocess.run([str(tool), "pack", "/o", "/d", str(lay), "/p", str(d / "fixture.msix")], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return r.returncode == 0, r.stdout + r.stderr


def t_validate(job, files, d):
    if os.name == "nt":
        ok, out = makeappx_pack(job, d)
        if ok is None:
            skip("msix: MakeAppx", out)
        else:
            check(ok, "msix: MakeAppx pack accepts the fixture manifest (file types, protocol, StartupTask)"
                  + ("" if ok else " -- " + out.strip()[-800:]))
    probe = pkg_linux.probe()
    if probe.get("error"):
        return skip("validate", f"Linux / WSL: {probe['error'][:200]}")
    run_inside("validate", lp(files["desktop"]), lp(files["metainfo"]), *[lp(f) for f in files["scriptlets"]])


def t_apps(d):
    """every apps/*/PACKAGE: its .desktop entry, metainfo and AppxManifest (with a stand-in
    program; MakeAppx pack checks file types and schemes against the schema), nothing built"""
    files = []
    apps = sorted(p.parent.name for p in (ROOT / "apps").glob("*/PACKAGE"))
    for app in apps:
        meta = pkgmeta.kv_file(ROOT / "apps" / app / "PACKAGE")
        aid, pkg = pkgmeta.app_id(meta, app), meta.get("deb_name", app.lower())
        a = d / "apps" / app
        a.mkdir(parents=True, exist_ok=True)
        (a / f"{aid}.desktop").write_bytes(pkgmeta.lf(pkgmeta.desktop_entry(meta, app, pkg, aid, pkg)))
        (a / f"{aid}.metainfo.xml").write_bytes(pkgmeta.lf(pkgmeta.metainfo(meta, app, aid, pkg)))
        files += [a / f"{aid}.desktop"] + (["--no-homepage"] if not meta.get("homepage") else []) + [a / f"{aid}.metainfo.xml"]
        if os.name != "nt" or pkg_msix.find_makeappx() is None:
            continue
        exe = a / f"{app}.exe"
        shutil.copyfile(sys.executable, exe)
        styles = meta.get("styles", "winui").split()
        job = pkgmeta.Job(app=app, meta=meta, styles=styles, root=ROOT, dist=a, work=a, impl="rs", pkg=pkg,
                          version=meta.get("version", "0.1.0"), exe={s: {"win": exe} for s in styles},
                          extra={"win": [], "linux": []}, bins={"win": [], "linux": []}, cross={}, notices={}, units=[])
        lay = a / "layout"
        with contextlib.redirect_stdout(io.StringIO()):
            pkg_msix.layout(job, job.default_style("win"), lay)
        r = subprocess.run([str(pkg_msix.find_makeappx()), "pack", "/o", "/d", str(lay), "/p", str(a / "app.msix")],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        n = len(pkgmeta.filetypes(meta))
        check(r.returncode == 0, f"apps: {app}: MakeAppx pack accepts its AppxManifest ({n} file types)"
              + ("" if r.returncode == 0 else " -- " + (r.stdout + r.stderr).strip()[-600:]))
    if pkg_linux.probe().get("error"):
        return skip("apps: validators", "Linux / WSL not reachable")
    run_inside("validate", *[f if isinstance(f, str) else lp(f) for f in files])
    if not QUIET:
        print(f"INFO apps: {', '.join(apps)}")


def t_runtime(rt):
    probe = pkg_linux.probe()
    if probe.get("error"):
        return skip("runtime", f"Linux / WSL: {probe['error'][:200]}")
    if not probe.get("mksquashfs"):
        return skip("runtime", "mksquashfs missing (squashfs-tools)")
    run_inside("runtime", lp(rt))


# ---------------------------------------------------------------- the packages of an app

def t_packages(app, out, skip_build):
    """python tools/package.py APP --formats all --out OUT, then each format"""
    dist = ROOT / out
    if not skip_build:
        r = subprocess.run([sys.executable, "tools/package.py", app, "--formats", "all", "--out", out], cwd=ROOT,
                           capture_output=True, text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
        (WORK / "package.log").write_text(r.stdout + r.stderr, encoding="utf-8")
        if not check(r.returncode == 0, f"package.py {app} --formats all (log: build/pkg_check/package.log)"):
            print((r.stdout + r.stderr)[-3000:])
            return
        for line in r.stdout.splitlines():
            if line.startswith("[formats]"):
                print("INFO " + line)
    meta = pkgmeta.kv_file(ROOT / "apps" / app / "PACKAGE")
    aid, pkg, ver = pkgmeta.app_id(meta, app), meta.get("deb_name", app.lower()), meta.get("version", "0.1.0")
    styles = meta.get("styles", "winui").split()
    perms = pkgmeta.permissions(app)
    scripts = sorted((ROOT / "apps" / app / "tests").glob("*.uis"))
    probe = pkg_linux.probe()

    # AppImage
    for s in styles:
        f = dist / f"{app}-{ver}-{s}-x86_64.AppImage"
        if not f.exists():
            skip(f"appimage {s}", "not built (see the [formats] lines)")
            continue
        b = f.read_bytes()
        end = pkg_linux.elf_end(b)
        check(b[:4] == b"\x7fELF" and b[8:11] == b"AI\x02" and b[end:end + 4] == b"hsqs" and ".upd_info" in pkg_linux.elf_sections(b),
              f"appimage {s}: ELF with AI\\x02, .upd_info, SquashFS right after the ELF data")
        run_inside("appimage", lp(f), *[lp(x) for x in scripts])

    # RPM
    spec = dist / "rpm" / "SPECS" / f"{pkg}.spec"
    tars = sorted((dist / "rpm" / "SOURCES").glob(f"{pkg}-*-root.tar.gz"))
    if check(spec.exists() and tars, "rpm: .spec and the root tarball written"):
        text = spec.read_text(encoding="utf-8")
        listed = sorted(m.group(1) for m in re.finditer(r'^"/([^"]+)"$', text, re.M))
        with tarfile.open(tars[-1]) as t:
            files = sorted(m.name[2:] for m in t.getmembers() if m.isfile())
        check(listed == files and f"Name:           {pkg}\n" in text, f"rpm: %files lists exactly the tarball's {len(files)} files")
        rpms = sorted(dist.glob(f"{pkg}-*.rpm"))
        if rpms:
            check(True, f"rpm: {rpms[-1].name} built")
        else:
            skip("rpm: .rpm", "rpmbuild " + ("found, but no package" if probe.get("rpmbuild") else "not installed (only the .spec)"))

    # Flatpak
    mf = dist / "flatpak" / f"{aid}.json"
    if check(mf.exists(), f"flatpak: manifest {mf.name} written"):
        man = json.loads(mf.read_text(encoding="utf-8"))
        args, _ = pkg_linux.finish_args(perms, meta.get("flatpak_args", "").split())
        check(man["id"] == aid and man["command"] == pkg and man["finish-args"] == args,
              f"flatpak: id, command, finish-args from PERMISSIONS ({' '.join(man['finish-args'])})")
        fd = dist / "flatpak" / "files"
        need = [f"lib/{pkg}/{pkg}", f"share/applications/{aid}.desktop", f"share/metainfo/{aid}.metainfo.xml",
                f"share/icons/hicolor/128x128/apps/{aid}.png"]
        check(all((fd / n).exists() for n in need), "flatpak: files/ holds the program, .desktop, metainfo, icons")
        if not probe.get("error"):
            run_inside("validate", lp(fd / need[1]), lp(fd / need[2]))
        bundle = dist / f"{aid}.flatpak"
        if bundle.exists():
            check(bundle.stat().st_size > 1000, f"flatpak: bundle {bundle.name} built ({bundle.stat().st_size} bytes)")
            run_inside("flatpak", lp(dist / "flatpak"), aid, lp(ROOT), *[lp(x) for x in scripts])
        else:
            skip("flatpak: bundle", "flatpak-builder or the runtime missing (see the [formats] lines)")

    # MSIX
    t_msix(app, meta, dist, ver)

    # AAB
    t_aab()


def t_msix(app, meta, dist, ver):
    f = dist / f"{app}-{ver}-x64.msix"
    if os.name != "nt":
        return skip("msix", "Windows only")
    if not f.exists():
        return skip("msix", "not built (see the [formats] lines)")
    names = pkg_msix.package_files(f)
    check(f"{app}.exe" in names and "AppxManifest.xml" in names and all(f"Assets/{n}" in names for n in pkg_msix.LOGOS),
          f"msix: {app}.exe, AppxManifest.xml and the logos are in the package")
    xml = zipfile.ZipFile(f).read("AppxManifest.xml").decode("utf-8")
    check(f'Name="{pkg_msix.identity_name(meta, app)}"' in xml and f'Publisher="{pkg_msix.publisher(meta)}"' in xml
          and f'Version="{pkg_msix.msix_version(ver)}"' in xml, "msix: identity of the package read back")
    tool = pkg_msix.find_makeappx()
    un = WORK / "msix-unpack"
    shutil.rmtree(un, ignore_errors=True)
    r = subprocess.run([str(tool), "unpack", "/p", str(f), "/d", str(un), "/o"], capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    check(r.returncode == 0 and (un / f"{app}.exe").exists(), "msix: MakeAppx unpack reads it again")
    # a test certificate whose subject is the Publisher: sign a copy, signtool verify reads it
    import codesign
    signtool = codesign.find_signtool()
    if signtool is None:
        return skip("msix: signing", "signtool.exe not found")
    d = WORK / "cert"
    d.mkdir(parents=True, exist_ok=True)
    pw = base64.b16encode(os.urandom(8)).decode()
    subj = pkg_msix.publisher(meta)
    pfx = test_cert(d, "test", "/" + subj.replace(", ", "/"), pw)
    if pfx is None:
        return skip("msix: signing", "no openssl (host or Linux / WSL) for the test certificate")
    import package
    saved = {k: os.environ.get(k) for k in ("jfgui_SIGN_PFX", "jfgui_SIGN_PFX_PASS", "jfgui_SIGN_THUMBPRINT", "jfgui_SIGN_TIMESTAMP")}
    os.environ.update(jfgui_SIGN_PFX=str(d / "test.pfx"), jfgui_SIGN_PFX_PASS=pw)
    for k in ("jfgui_SIGN_THUMBPRINT", "jfgui_SIGN_TIMESTAMP"):
        os.environ.pop(k, None)
    try:
        signed = WORK / f.name
        shutil.copyfile(f, signed)
        s = package.Signing(False, True)
        try:
            s.msix(signed, subj)
            check("AppxSignature.p7x" in zipfile.ZipFile(signed).namelist(), f"msix: signed with a test certificate ({subj})")
        except RuntimeError as e:
            return check(False, f"msix: signed with a test certificate -- {e}")
        v = subprocess.run([str(signtool), "verify", "/pa", "/v", str(signed)], capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
        out = v.stdout + v.stderr
        check(f"Issued to: {subj.split('=', 1)[1].split(',')[0]}" in out and "No signature found" not in out,
              "msix: signtool verify reads the signature (the test root is untrusted: expected)")
        # a certificate of another subject: signtool refuses it (its own reason), the message names the Publisher
        other = test_cert(d, "other", "/CN=Someone Else (untrusted)", pw)
        if other is not None:
            os.environ["jfgui_SIGN_PFX"] = str(other)
            wrong = WORK / ("wrong-" + f.name)
            shutil.copyfile(f, wrong)
            try:
                package.Signing(False, True).msix(wrong, subj)
                check(False, "msix: a certificate of another subject is refused")
            except RuntimeError as e:
                check("0x8007000b" in str(e).lower() and f"Publisher des Pakets ({subj}" in str(e),
                      "msix: a certificate of another subject is refused (0x8007000B), the message names the Publisher"
                      + ("" if "0x8007000b" in str(e).lower() else f" -- {str(e).strip()[-300:]}"))
            wrong.unlink(missing_ok=True)
        signed.unlink(missing_ok=True)
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def test_cert(d, name, subject, pw):
    """a self-signed code-signing certificate for this run (never installed) -> d/<name>.pfx or
    None.  Made by the host's openssl when there is one (the clock signtool checks the start
    time against), else in Linux / WSL, then waiting until its start time has passed here (the
    WSL clock may run ahead)"""
    import time
    from tools.host_exec import run_linux
    req = ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", subject,
           "-addext", "extendedKeyUsage=codeSigning", "-addext", "keyUsage=critical,digitalSignature"]
    key, cert, pfx = d / f"{name}.key.pem", d / f"{name}.cert.pem", d / f"{name}.pfx"
    host = shutil.which("openssl")
    if host:
        run = lambda args: subprocess.run([host, *args], capture_output=True, stdin=subprocess.DEVNULL).returncode  # noqa: E731
        path = str
    else:
        run = lambda args: run_linux(["openssl", *args]).returncode  # noqa: E731
        path = lp
    if run(req + ["-keyout", path(key), "-out", path(cert)]) or \
            run(["pkcs12", "-export", "-out", path(pfx), "-inkey", path(key), "-in", path(cert), "-passout", f"pass:{pw}"]):
        return None
    if not host:
        ahead = float(run_linux(["date", "+%s.%N"]).stdout.strip() or 0) - time.time()
        if 0 < ahead < 300:
            time.sleep(ahead + 1)
    return pfx


def t_aab():
    """what tools/android.py would need for an Android App Bundle (only reported)"""
    import android
    found = shutil.which("bundletool") or os.environ.get("jfgui_BUNDLETOOL")
    try:
        sdk, _, _, build, _, java, _ = android.sdk_tools()
        aapt2 = (build / ("aapt2.exe" if os.name == "nt" else "aapt2")).exists()
        what = f"SDK {sdk}, build-tools {build.name}, aapt2 {'ja' if aapt2 else 'nein'}, java {java}"
    except RuntimeError as e:
        what = str(e)
    skip("aab", f"not implemented (docs/PACKAGING.md \"Android App Bundle\"); bundletool {'found: ' + found if found else 'missing'}; {what}")


def t_notices():
    """THIRD_PARTY_NOTICES.md names every file of licenses/; every package, build and APK gets it
    with licenses/ and, per style, the notices of the styles whose fonts or icons it embeds"""
    import build_rs
    text = (ROOT / "THIRD_PARTY_NOTICES.md").read_text(encoding="utf-8")
    files = sorted(p.name for p in (ROOT / "licenses").iterdir() if p.is_file())
    missing = [n for n in files if f"`{n}`" not in text]
    check(files and not missing, f"notices: THIRD_PARTY_NOTICES.md names the {len(files)} files of licenses/"
          + (f" -- missing: {', '.join(missing)}" if missing else ""))
    styles = sorted(p.name for p in (ROOT / "styles").iterdir() if (p / "lib.rs").exists())
    shipped = build_rs.app_notices(styles)
    need = [Path("THIRD_PARTY_NOTICES.md"), Path("licenses/text_shape/NOTICE.md")] + [Path("licenses") / n for n in files]
    check(all(n in shipped for n in need), f"notices: app_notices ships THIRD_PARTY_NOTICES.md, licenses/ and text_shape")
    lost = [f"{s} -> {r.as_posix()}" for s in styles for o in build_rs.embedded_styles(s)
            for r in build_rs.own_style_notices(o) if r not in build_rs.style_notices(s)]
    check(not lost, "notices: a style ships the notices of the styles it embeds"
          + (f" -- missing: {', '.join(lost)}" if lost else ""))
    jfoffice = Path("licenses/jfoffice/LICENSE-icons.txt")
    check(all(jfoffice in build_rs.style_notices(s) for s in ("backupcraft", "netlab"))
          and all(Path(f"licenses/{s}/DejaVu-LICENSE.txt") in build_rs.style_notices(s)
                  for s in ("kde", "classic")),
          "notices: backupcraft and netlab ship WordCraft's MIT notice, kde and classic the DejaVu license")


def main():
    global QUIET
    if len(sys.argv) > 2 and sys.argv[1] == "--inside-linux":
        return inside_main(sys.argv[2:])
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--quick", action="store_true", help="metadata, validators and the runtime only (the test suite)")
    ap.add_argument("--runtime", help="the AppImage runtime to check (default build/linux/appimagert when built)")
    ap.add_argument("--app", default="template", help="the app to package (without --quick; default template)")
    ap.add_argument("--out", default="build/pkg_check/dist", help="package.py --out (without --quick)")
    ap.add_argument("--no-build", action="store_true", help="check the packages already in --out")
    ap.add_argument("--keep", action="store_true", help="keep build/pkg_check")
    ap.add_argument("--summary", action="store_true", help="print only FAIL / SKIP lines and the result")
    a = ap.parse_args()
    QUIET = a.summary
    meta_dir = WORK / "meta"
    shutil.rmtree(meta_dir, ignore_errors=True)
    meta_dir.mkdir(parents=True)
    job, files = t_meta(meta_dir)
    t_missing(meta_dir)
    t_elf()
    t_validate(job, files, meta_dir)
    t_apps(meta_dir)
    t_notices()
    rt = Path(a.runtime) if a.runtime else ROOT / "build" / "linux" / "appimagert"
    if rt.exists():
        t_runtime(rt)
    else:
        skip("runtime", f"{rt.relative_to(ROOT) if rt.is_relative_to(ROOT) else rt} not built (python build_rs.py app appimagert --target linux)")
    if not a.quick:
        t_packages(a.app, a.out, a.no_build)
    if not a.keep:
        shutil.rmtree(meta_dir, ignore_errors=True)
        for p in (WORK / "cert", WORK / "msix-unpack"):
            shutil.rmtree(p, ignore_errors=True)
    print(f"pkg_check: {'FAIL (' + str(len(FAILS)) + ')' if FAILS else 'ok'}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
