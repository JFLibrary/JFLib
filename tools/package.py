"""package.py -- installable packages of an app, one build per style:

  build/dist/<App>_Setup.exe          Windows: the setup stub + compressed payload (every style
                                      variant, a preview of each); the user picks the look, the
                                      setup installs only that exe (folder, desktop / start menu
                                      shortcuts, "installed apps" entry, uninstall.exe)
  build/dist/<deb>_<ver>_amd64.deb    Linux: every variant in /usr/lib/<deb>/, debconf asks for
                                      the look (dpkg-reconfigure <deb> changes it), the choice is
                                      an update-alternatives link /usr/bin/<deb>; .desktop + icons

  python tools/package.py PaintCraft [--styles winui,kde] [--no-deb] [--no-win] [--out build/dist]
  python tools/package.py PaintCraft --impl asm [--out build/dist_asm]       (legacy mode)
  python tools/package.py PaintCraft [--sign] [--repo build/repo] [--update build/channel
                                     [--update-url https://host/channel] [--update-key FILE]]
  python tools/package.py PaintCraft --formats setup,deb,appimage,rpm,flatpak,msix | all

--formats (default setup,deb) adds the other formats (tools/pkg_linux.py, tools/pkg_msix.py,
docs/PACKAGING.md):
  build/dist/<App>-<ver>-<style>-x86_64.AppImage   per style: the AppDir as SquashFS (mksquashfs
                                      -comp zstd) behind the runtime apps/appimagert, which
                                      extracts it on the first start (no FUSE);
                                      --appimage-runtime FILE: another runtime
  build/dist/rpm/SPECS/<deb>.spec     + SOURCES/<deb>-<ver>-root.tar.gz; the .rpm by rpmbuild
                                      when it is installed (every style, update-alternatives)
  build/dist/flatpak/<app id>.json    the flatpak-builder manifest (finish-args from PERMISSIONS)
                                      + files/; build/dist/<app id>.flatpak by flatpak-builder
  build/dist/<App>-<ver>-x64.msix     MakeAppx of the default Windows style (AppxManifest.xml:
                                      file types, URL schemes, StartupTask), signed like the setup;
                                      build/dist/<App>.appinstaller with PACKAGE appinstaller_url=
A missing tool skips that package and says what it wrote instead; --no-win / --no-deb drop
every Windows / Linux format.

Signing (tools/codesign.py, docs/PACKAGING.md "Signing"): with a certificate in the environment
(jfgui_SIGN_PFX + jfgui_SIGN_PFX_PASS or jfgui_SIGN_THUMBPRINT, optional jfgui_SIGN_TIMESTAMP)
signtool signs every Windows program variant, its modules and the finished setup (Authenticode;
the setup finds its payload before the signature).  --repo DIR writes a flat apt repository with
the .deb, signed with jfgui_GPG_KEY (InRelease, Release.gpg, repo.gpg); the same key signs the
AppImages (.sha256_sig / .sig_key, as appimagetool --sign) and the Flatpak repository and bundle.
--update DIR writes a signed update channel per style for lib/update (DIR/<style>/update.json +
.sig; the program, the .deb and the AppImage of the style; the key from --update-key,
jfgui_UPDATE_KEY_FILE or jfgui_UPDATE_KEY).  A missing tool or key skips that signature and
says so; --sign makes it an error.

--impl rs (the default) packages the Rust build: every variant is built by one
`build_rs.py app NAME --target both --style S1,S2,.. --profile size` (one shared output folder
<out>/rs_<profile>: the libraries once, both targets side by side, the style variants linked in
parallel; each exe copied to <out>/<App>/<style>/<target>), the setup stub is apps/installer.
--profile size (default; rewrite/REPORT.md section 2: -14 to -21 % against opt-level 3, codecs / effects / rendering
unchanged, UI scripts about +4 %), o2 (opt-level 2, no LTO) or release (build_rs's default
release: opt-level 2, fat LTO) picks the build_rs release variant.  --impl asm packages the
frozen assembly reference instead (legacy/build.py, legacy/apps/installer); the payload, the
previews and the .deb are made the same way for both.

Inputs: apps/<App>/PACKAGE (key=value: display, version, publisher, summary, description,
generic, categories, section, deb_name, default_win, default_linux, styles; optional
cross_bins= and unit_<name>=, below; filetypes= / schemes=: the file types and URL schemes the
app opens -- the setup registers them under HKCU\\Software\\Classes, the .deb in its .desktop
file and, for "own" types, a shared-mime-info package), apps/<App>/res/app.ico and
app_{48,128,256}.png (tools/mkicon.py), styles/<style>/INFO (shown name, description;
--impl asm: legacy/ASMLIB/styles/<style>/INFO). The .deb uses local dpkg-deb on Linux, or
WSL on Windows (jfgui_WSL_DISTRO, default Ubuntu-24.04). Payload layout:
docs/PACKAGING.md (apps/installer/payload.rs reads it).

jxl-module: the loadable modules an app uses (build_rs.py APP_MODULES; PaintCraft: jxlenc, the
JPEG XL writer) are built with its variants (one file for every style) and installed next to
the exe: a PK_FILE entry of the setup (installed into the program folder, listed in
install.ini, removed by uninstall.exe) and /usr/lib/<deb>/ in the .deb (the program finds them
next to its real path).

app-bins: cross_bins=NAME .. puts the other system's build of these BINS beside the program,
agents/linux-x86_64/NAME in the setup and agents/windows-x86_64/NAME.exe in the .deb (BackupCraft:
a Windows server installs the Linux agent over SSH).  unit_<name>=<description>|<BIN> <args>
ships the systemd unit /lib/systemd/system/<name>.service in the .deb (ExecStart=/usr/lib/<deb>/
<BIN> <args>); it is not enabled -- systemctl enable --now <name> starts it.
"""
import argparse
import os
import shutil
import struct
import subprocess
import sys
import zipfile
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import assetpack  # noqa: E402
import codesign  # noqa: E402
import pkg_linux  # noqa: E402
import pkg_msix  # noqa: E402
import pkgmeta  # noqa: E402
import pngutil  # noqa: E402
import update_manifest  # noqa: E402
from pkgmeta import desktop_entry, filetypes, kv_file, lf, mime_package, schemes  # noqa: E402,F401  (also package.kv_file)

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from build_rs import APP_MODULES, module_file, app_notices  # noqa: E402  (jxl-module)
from build_rs import app_bin_files  # noqa: E402  (app-bins)
from tools.host_exec import linux_path, run_linux
STYLES = {"rs": ROOT / "styles", "asm": ROOT.parent / "legacy" / "ASMLIB" / "styles"}  # <style>/INFO
WSL = ["wsl.exe", "-d", "Ubuntu-24.04"]
PREVIEW = (1100, 700)           # rendered size (DIP, 100 %)
THUMB = (480, 305)              # shown in the setup (px)

PK_MANIFEST, PK_STYLE, PK_PREVIEW, PK_FILE = 1, 2, 3, 5
PF_ZLIB = 1


def style_info(style, impl="rs"):
    f = STYLES[impl] / style / "INFO"
    lines = f.read_text(encoding="utf-8").splitlines() if f.exists() else []
    return (lines[0] if lines else style), (lines[1] if len(lines) > 1 else "")


def run(cmd, **kw):
    print("  $", " ".join(str(c) for c in cmd))
    # Packaging is noninteractive. A manager worker keeps a blocking read on its
    # lifetime pipe; inheriting that same Windows handle can block Python's
    # standard-stream initialization in nested build processes.
    kw.setdefault("stdin", subprocess.DEVNULL)
    subprocess.run([str(c) for c in cmd], check=True, cwd=ROOT, **kw)


def build_variant(app, style, out, targets):
    for t in targets:
        run([sys.executable, str(ROOT.parent / "legacy" / "build.py"), "app", app, "--target", t, "--style", style,
             "--out", str(ROOT / out)])


def exe_name(app, target):
    return app + (".exe" if target == "win" else "")


def rs_mode(profile):
    """build_rs.py arguments of a --profile choice"""
    return ["--release"] if profile == "release" else ["--profile", profile]


def build_variants_rs(app, styles, work, targets, rsout, profile):
    """--impl rs: one build_rs.py run into the shared rsout for every style and target (the
    libraries built once, both targets side by side, the style variants linked in parallel:
    <rsout>/<t>/<style>/ for several styles), each exe copied to work/<style>/<target>/ as
    build.py would place it (jxl-module: and the app's modules next to it)"""
    tgt = "both" if len(targets) == 2 else targets[0]
    run([sys.executable, "build_rs.py", "app", app, "--target", tgt, "--style", ",".join(styles),
         *rs_mode(profile), "--out", rsout])
    for s in styles:
        for t in targets:
            src = ROOT / rsout / t / s if len(styles) > 1 else ROOT / rsout / t
            (ROOT / work / s / t).mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src / exe_name(app, t), ROOT / work / s / t / exe_name(app, t))
            for m in app_modules(app, t, "rs") + app_bins(app, t, "rs"):
                shutil.copyfile(ROOT / rsout / t / m, ROOT / work / s / t / m)


def app_modules(app, target, impl):
    """(jxl-module) the module files of the app next to its exe (none for --impl asm)"""
    return [module_file(m, target) for m in APP_MODULES.get(app, [])] if impl == "rs" else []


def app_bins(app, target, impl):
    """(app-bins) the app's further executables (apps/<App>/BINS), one file for every style,
    installed next to its exe (none for --impl asm)"""
    return app_bin_files(app, target) if impl == "rs" else []


def package_notices(styles, impl):
    """The library's third-party notices and every bundled style's font/artwork notices, also
    when the user selects another style."""
    if impl != "rs":
        return {}
    return app_notices(styles)


def build_stub(impl, work, rsout, profile="size"):
    """the setup stub (apps/installer or apps/installer) -> its bytes"""
    if impl == "rs":
        run([sys.executable, "build_rs.py", "app", "installer", "--target", "win", *rs_mode(profile),
             "--out", rsout])
        return (ROOT / rsout / "win" / "installer.exe").read_bytes()
    stub_out = work / "setup"
    run([sys.executable, str(ROOT.parent / "legacy" / "build.py"), "app", "installer", "--target", "win",
         "--out", str(ROOT / stub_out)])
    return (ROOT / stub_out / "win" / "installer.exe").read_bytes()


def preview(exe, png_out):
    env = dict(os.environ, PaintCraft_CONFIG_DIR=str(ROOT / "build" / "test_cfg"))
    run([exe, "--shot", png_out, "--size", f"{PREVIEW[0]}x{PREVIEW[1]}", "--theme", "light",
         "--anim", "off"], env=env, capture_output=True)
    w, h, px = pngutil.read_png(png_out)
    small = pngutil.downscale(w, h, px, *THUMB)
    return pngutil.png_bytes(THUMB[0], THUMB[1], small)


def payload(entries):
    """entries: list of (kind, flags, name, label, data, usize) -> payload bytes"""
    head = b"AIPK" + struct.pack("<I", len(entries))
    recs, blobs = b"", b""
    base = 8 + 128 * len(entries)
    for kind, flags, name, label, data, usize in entries:
        nb, lb = name.encode("utf-8")[:47], label.encode("utf-8")[:47]
        recs += struct.pack("<IIIIQQ", kind, len(data), usize, flags, base + len(blobs), 0)
        recs += nb.ljust(48, b"\0") + lb.ljust(48, b"\0")
        blobs += data
    return head + recs + blobs


def make_setup(app, meta, styles, dist, work, impl="asm", rsout=None, profile="size"):
    print("[win] setup")
    stub = build_stub(impl, work, rsout, profile)
    man = [f"app={app}", f"display={meta.get('display', app)}", f"version={meta.get('version', '')}",
           f"publisher={meta.get('publisher', '')}", f"default={meta.get('default_win', styles[0])}"]
    entries = []
    for s in styles:
        label, desc = style_info(s, impl)
        man.append(f"desc_{s}={desc}")
    if filetypes(meta):                                  # checked here, registered by the setup
        man.append(f"filetypes={meta['filetypes']}")
    if schemes(meta):
        man.append(f"schemes={' '.join(schemes(meta))}")
    man.append(f"app_id={pkgmeta.app_id(meta, app)}")   # the AppUserModelID of shortcuts, pins, ProgIDs
    mtext = ("\n".join(man) + "\n").encode("utf-8")
    entries.append((PK_MANIFEST, PF_ZLIB, "manifest", "", zlib.compress(mtext, 9), len(mtext)))
    for s in styles:
        label, _ = style_info(s, impl)
        exe = (ROOT / work / s / "win" / f"{app}.exe").read_bytes()
        entries.append((PK_STYLE, PF_ZLIB, s, label, zlib.compress(exe, 9), len(exe)))
        png = preview(ROOT / work / s / "win" / f"{app}.exe", ROOT / work / f"preview_{s}.png")
        entries.append((PK_PREVIEW, 0, s, label, png, len(png)))
    # jxl-module: the app's modules (the same file for every style) next to the exe
    for m in app_modules(app, "win", impl) + app_bins(app, "win", impl):     # app-bins: installed alike
        data = (ROOT / work / styles[0] / "win" / m).read_bytes()
        entries.append((PK_FILE, PF_ZLIB, m, "", zlib.compress(data, 9), len(data)))
    for name, data in cross_files(app, meta, rsout, "linux"):    # app-bins: the other system's build
        entries.append((PK_FILE, PF_ZLIB, name, "", zlib.compress(data, 9), len(data)))
    # notices whose path is longer than a payload name (47 bytes) go, complete and with their
    # paths as headings, into one ALL-LICENSES.txt of their library folder
    combined = {}
    for relative, source in package_notices(styles, impl).items():
        name = relative.as_posix()
        if len(name.encode("utf-8")) > 47:
            combined.setdefault("/".join(name.split("/")[:2]), []).append((name, source))
            continue
        data = source.read_bytes()
        entries.append((PK_FILE, PF_ZLIB, name, "", zlib.compress(data, 9), len(data)))
    for folder, files in sorted(combined.items()):
        name = f"{folder}/ALL-LICENSES.txt"
        if len(name.encode("utf-8")) > 47:
            raise ValueError(f"Installer notice path exceeds the payload limit: {name}")
        data = b"".join(f"==== {n} ====\n\n".encode("utf-8") + s.read_bytes() + b"\n\n" for n, s in files)
        entries.append((PK_FILE, PF_ZLIB, name, "", zlib.compress(data, 9), len(data)))
    pay = payload(entries)
    trailer = b"ASMINST1" + struct.pack("<QQ", len(stub), len(pay))
    out = ROOT / dist / f"{app}_Setup.exe"
    out.write_bytes(stub + pay + trailer)
    print(f"[win] {out.relative_to(ROOT)}: {out.stat().st_size} bytes "
          f"({len(styles)} styles, stub {len(stub)}, payload {len(pay)})")
    return out


def cross_files(app, meta, rsout, other):
    """(app-bins) cross_bins=: the BINS built for the other system -> [(agents/<os>/file, bytes)]"""
    out = []
    names = meta.get("cross_bins", "").split()
    if not names:
        return out
    folder = "linux-x86_64" if other == "linux" else "windows-x86_64"
    for n in names:
        f = exe_name(n, other)
        src = ROOT / rsout / other / f if rsout else None
        if src is None or not src.exists():
            print(f"  (cross_bins: {f} for {other} is not built -- package without it)")
            continue
        out.append((f"agents/{folder}/{f}", src.read_bytes()))
    return out


def unit_files(pkg, meta):
    """unit_<name>= -> [(name, unit text)] with the program in /usr/lib/<pkg>/ (the format of
    plat::service::unit_text)"""
    out = []
    for k, v in sorted(meta.items()):
        if not k.startswith("unit_") or "|" not in v:
            continue
        desc, cmd = (x.strip() for x in v.split("|", 1))
        exe, _, args = cmd.partition(" ")
        out.append((k[5:], "[Unit]\nDescription=" + desc + "\nAfter=network-online.target\nWants=network-online.target\n\n"
                    "[Service]\nType=simple\nExecStart=/usr/lib/" + pkg + "/" + exe + (" " + args if args else "") + "\n"
                    "Restart=on-failure\nRestartSec=5\nUMask=0077\n\n[Install]\nWantedBy=multi-user.target\n"))
    return out


def wslpath(p):
    r = subprocess.run(WSL + ["-e", "wslpath", "-a", str(Path(p).resolve()).replace("\\", "/")],
                       capture_output=True, text=True, check=True)
    return r.stdout.strip()


def make_deb(app, meta, styles, dist, work, impl="rs", rsout=None):
    print("[linux] deb")
    pkg = meta.get("deb_name", app.lower())
    ver = meta.get("version", "0.1.0")
    display = meta.get("display", app)
    default = meta.get("default_linux", styles[0])
    tree = ROOT / work / "deb" / pkg
    if tree.exists():
        shutil.rmtree(tree)
    lib = tree / "usr" / "lib" / pkg
    lib.mkdir(parents=True)
    size = 0
    for s in styles:
        src = ROOT / work / s / "linux" / app
        shutil.copyfile(src, lib / f"{app}-{s}")
        size += src.stat().st_size
    for m in app_modules(app, "linux", impl):     # jxl-module: next to every variant
        src = ROOT / work / styles[0] / "linux" / m
        shutil.copyfile(src, lib / m)
        size += src.stat().st_size
    bins = app_bins(app, "linux", impl)           # app-bins: /usr/lib/<deb>/BIN, linked as /usr/bin/BIN
    for m in bins:
        src = ROOT / work / styles[0] / "linux" / m
        shutil.copyfile(src, lib / m)
        size += src.stat().st_size
    for name, data in cross_files(app, meta, rsout, "win"):     # app-bins: the other system's build
        (lib / name).parent.mkdir(parents=True, exist_ok=True)
        (lib / name).write_bytes(data)
        size += len(data)
    units = unit_files(pkg, meta)
    for name, text in units:
        d = tree / "lib" / "systemd" / "system"
        d.mkdir(parents=True, exist_ok=True)
        (d / f"{name}.service").write_bytes(lf(text))
    for relative, source in package_notices(styles, impl).items():
        destination = tree / "usr" / "share" / "doc" / pkg / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        size += source.stat().st_size
    apps = tree / "usr" / "share" / "applications"
    apps.mkdir(parents=True)
    (apps / f"{pkg}.desktop").write_bytes(lf(desktop_entry(meta, app, pkg, pkg)))
    own = [t for t in filetypes(meta) if t[3]]
    if own:
        # the app's own types: a shared-mime-info package (dpkg's trigger updates the database)
        d = tree / "usr" / "share" / "mime" / "packages"
        d.mkdir(parents=True)
        (d / f"{pkg}.xml").write_bytes(lf(mime_package(own)))
    for sz in (48, 128, 256):
        d = tree / "usr" / "share" / "icons" / "hicolor" / f"{sz}x{sz}" / "apps"
        d.mkdir(parents=True)
        (d / f"{pkg}.png").write_bytes(pkgmeta.icon(app, sz))     # res/app_<sz>.png (or scaled, or a placeholder)
    deb = tree / "DEBIAN"
    deb.mkdir()
    labels = [style_info(s, impl)[0] for s in styles]
    desc = meta.get("description", "")
    (deb / "control").write_bytes(lf(
        f"Package: {pkg}\nVersion: {ver}\nArchitecture: amd64\nSection: {meta.get('section', 'graphics')}\nPriority: optional\n"
        f"Maintainer: jfguiLib Test <test@localhost>\nInstalled-Size: {size // 1024 + 64}\n"
        "Depends: debconf (>= 0.5) | debconf-2.0\n"
        f"Description: {meta.get('summary', display)}\n {desc}\n .\n"
        f" Enthaelt das Programm in {len(styles)} Stilen; die Auswahl erfolgt bei der Installation.\n"))
    (deb / "templates").write_bytes(lf(
        f"Template: {pkg}/style\nType: select\nChoices-C: {', '.join(styles)}\n"
        f"Choices: {', '.join(labels)}\nDefault: {default}\n"
        f"Description: Aussehen von {display}:\n"
        " Der gewählte Stil ist fest in das Programm eingebaut. Ändern lässt er sich\n"
        f" später mit „sudo dpkg-reconfigure {pkg}“.\n"))
    (deb / "config").write_bytes(lf(
        "#!/bin/sh\nset -e\n. /usr/share/debconf/confmodule\n"
        f"db_input high {pkg}/style || true\ndb_go || true\nexit 0\n"))
    alts = "\n".join(
        f"  update-alternatives --install /usr/bin/{pkg} {pkg} /usr/lib/{pkg}/{app}-{s} "
        f"{50 if s == default else 10}" for s in styles)
    (deb / "postinst").write_bytes(lf(
        "#!/bin/sh\nset -e\n. /usr/share/debconf/confmodule\n"
        'if [ "$1" = "configure" ]; then\n'
        f"{alts}\n"
        f"  RET={default}\n  db_get {pkg}/style || true\n"
        f'  [ -x "/usr/lib/{pkg}/{app}-$RET" ] || RET={default}\n'
        f'  update-alternatives --set {pkg} "/usr/lib/{pkg}/{app}-$RET"\n'
        + "".join(f"  ln -sf /usr/lib/{pkg}/{m} /usr/bin/{m}\n" for m in bins) +
        ("  if [ -d /run/systemd/system ]; then systemctl daemon-reload || true; fi\n" if units else "") +
        "  if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q || true; fi\n"
        + ("  if command -v update-mime-database >/dev/null 2>&1; then update-mime-database /usr/share/mime || true; fi\n"
           if own else "") +
        "  if command -v gtk-update-icon-cache >/dev/null 2>&1; then "
        "gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true; fi\n"
        "fi\ndb_stop\nexit 0\n"))
    rm_alts = "\n".join([f"  update-alternatives --remove {pkg} /usr/lib/{pkg}/{app}-{s} || true"
                         for s in styles] + [f"  rm -f /usr/bin/{m}" for m in bins]
                        + [f"  if [ -d /run/systemd/system ]; then systemctl disable --now {u} >/dev/null 2>&1 || true; fi" for u, _ in units])
    (deb / "prerm").write_bytes(lf(
        "#!/bin/sh\nset -e\n"
        'if [ "$1" = "remove" ] || [ "$1" = "deconfigure" ]; then\n'
        f"{rm_alts}\nfi\nexit 0\n"))
    (deb / "postrm").write_bytes(lf(
        "#!/bin/sh\nset -e\n"
        'if [ "$1" = "purge" ] && [ -e /usr/share/debconf/confmodule ]; then\n'
        "  . /usr/share/debconf/confmodule\n  db_purge\nfi\nexit 0\n"))
    # build inside the Linux file system (permissions), copy the result back
    out = ROOT / dist / f"{pkg}_{ver}_amd64.deb"
    result = run_linux(["python3", linux_path(ROOT / "tools/linux_task.py"), "deb", linux_path(tree), linux_path(out)])
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    print(f"[linux] {out.relative_to(ROOT)}: {out.stat().st_size} bytes ({len(styles)} styles)")
    return out


class Signing:
    """what package.py signs and what it could not (docs/PACKAGING.md, "Signing"); --sign turns
    every gap into an error"""

    def __init__(self, required, windows):
        self.required, self.done, self.skipped = required, [], []
        self.pe, why = codesign.authenticode() if windows else (None, None)
        if why:
            self.skip(why)

    def skip(self, why):
        if self.required:
            raise SystemExit(f"--sign: {why}")
        self.skipped.append(why)
        print(f"  [sign] übersprungen: {why}")

    def windows(self, paths, desc=None):
        """Authenticode for these files (programs, modules, the setup)"""
        if self.pe and paths:
            self.pe.sign(paths, desc)
            self.done += [f"Authenticode ({self.pe.what}): {Path(p).name}" for p in paths]

    def msix(self, path, publisher):
        """an MSIX package (signtool writes AppxSignature.p7x; the certificate's subject must be
        the manifest's Publisher)"""
        try:
            self.pe.sign([path], pe=False)
        except RuntimeError as e:
            if "0x8007000b" not in str(e).lower():       # signtool's code for "subject is not the Publisher"
                raise
            raise RuntimeError(f"{e}\nMSIX: der Antragsteller des Zertifikats ist nicht der Publisher des Pakets "
                               f"({publisher}; PACKAGE msix_publisher= oder jfgui_MSIX_PUBLISHER)") from None
        if "AppxSignature.p7x" not in zipfile.ZipFile(path).namelist():
            raise RuntimeError(f"signtool meldet Erfolg, aber {path} hat keine AppxSignature.p7x")
        self.done.append(f"MSIX ({self.pe.what}): {Path(path).name}")

    def report(self):
        print("[sign] signiert: " + (", ".join(self.done) if self.done else "nichts"))
        for why in self.skipped:
            print(f"[sign] nicht signiert: {why}")


def sign_variants(app, styles, work, signing, impl):
    """Authenticode for every Windows exe variant and the files installed next to it"""
    files = [ROOT / work / s / "win" / f"{app}.exe" for s in styles]
    files += [ROOT / work / s / "win" / m for s in styles for m in app_modules(app, "win", impl) + app_bins(app, "win", impl)]
    signing.windows([f for f in files if f.exists()])


def update_channel(app, meta, styles, work, out, deb, base_url, key_file, signing, impl, appimages=None):
    """--update OUT: a signed channel per style (OUT/<style>/update.json + .sig, lib/update):
    Windows the program (kind exe) or, with modules / further programs, a bundle of the program
    folder (kind bundle); Linux the .deb (kind package: the package manager installs it) and the
    style's AppImage (kind appimage: an AppImage replaces its own file; appimages {style: path})"""
    seed = update_manifest.load_key(key_file)
    if seed is None:
        signing.skip("Update-Kanal: kein Signierschlüssel (--update-key DATEI, jfgui_UPDATE_KEY_FILE oder jfgui_UPDATE_KEY; "
                     "erzeugen mit python tools/update_manifest.py keygen DATEI)")
        return
    version = meta.get("version", "0.1.0")
    app_id = meta.get("update_app", app)
    for s in styles:
        files = []
        win = ROOT / work / s / "win"
        if (win / f"{app}.exe").exists():
            extra = app_modules(app, "win", impl) + app_bins(app, "win", impl)
            if extra:
                bundle = assetpack.pack_files({f"{app}.exe": (win / f"{app}.exe").read_bytes(),
                                               **{m: (win / m).read_bytes() for m in extra}}, compress=False)
                tmp = ROOT / work / f"{app}-{version}-{s}-win.jfa"
                tmp.write_bytes(bundle)
                files.append(("windows-x86_64", "bundle", tmp, tmp.name))
            else:
                files.append(("windows-x86_64", "exe", win / f"{app}.exe", f"{app}-{version}-{s}.exe"))
        if deb:
            files.append(("linux-x86_64", "package", deb, Path(deb).name))
        if (appimages or {}).get(s):
            files.append(("linux-x86_64", "appimage", appimages[s], Path(appimages[s]).name))
        if not files:
            continue
        m = update_manifest.write_channel(ROOT / out / s, app_id, version, files, seed, notes=meta.get("update_notes", ""),
                                          expires_days=int(meta.get("update_expires_days", "0")),
                                          base_url=(base_url.rstrip("/") + "/" + s) if base_url else "")
        signing.done.append(f"Update-Kanal ({s}): {m.relative_to(ROOT)} (Schlüssel {update_manifest.public_key(seed).hex()[:16]}…)")
        print(f"[update] {m.relative_to(ROOT)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("app")
    ap.add_argument("--styles", help="comma-separated (default: PACKAGE styles=)")
    ap.add_argument("--out", help="default build/dist (--impl asm: build/dist_asm)")
    ap.add_argument("--impl", choices=["rs", "asm"], default="rs",
                    help="package the Rust build (build_rs.py, the default) or the frozen assembly "
                         "reference (legacy/build.py)")
    ap.add_argument("--profile", choices=["size", "o2", "release"], default="size",
                    help="--impl rs: the build_rs release variant (default size, see rewrite/REPORT.md)")
    ap.add_argument("--no-win", action="store_true")
    ap.add_argument("--no-deb", action="store_true")
    ap.add_argument("--sign", action="store_true",
                    help="every signature is required: fail instead of skipping one (Authenticode, apt repository, "
                         "update channel) when its tool or key is missing")
    ap.add_argument("--repo", help="also write a flat apt repository with the .deb here (signed with jfgui_GPG_KEY)")
    ap.add_argument("--update", help="also write a signed update channel per style here (OUT/<style>/update.json)")
    ap.add_argument("--update-url", default="", help="--update: the channel's public address (absolute file URLs; "
                                                      "default: names relative to update.json)")
    ap.add_argument("--update-key", help="--update: the signing key file (default jfgui_UPDATE_KEY_FILE / jfgui_UPDATE_KEY)")
    ap.add_argument("--formats", default="setup,deb",
                    help="comma-separated: " + ",".join(FORMATS) + " or all (default setup,deb)")
    ap.add_argument("--appimage-runtime", help="AppImage: this runtime file instead of apps/appimagert "
                                               "(default jfgui_APPIMAGE_RUNTIME)")
    a = ap.parse_args()
    formats = formats_of(a.formats, a.no_win, a.no_deb)
    meta = kv_file(ROOT / "apps" / a.app / "PACKAGE")
    styles = (a.styles.split(",") if a.styles else meta.get("styles", "winui").split())
    dist = Path(a.out or ("build/dist_asm" if a.impl == "asm" else "build/dist"))
    work = dist / a.app
    rsout = dist / f"rs_{a.profile}"    # --impl rs: build_rs.py's output, shared by every app / style
    (ROOT / dist).mkdir(parents=True, exist_ok=True)
    targets = [t for t in ("win", "linux") if any(FORMATS[f] == t for f in formats)]
    signing = Signing(a.sign, "win" in targets)
    if a.impl == "rs":
        print(f"[{', '.join(styles)}] build")
        build_variants_rs(a.app, styles, work, targets, rsout, a.profile)
    else:
        for s in styles:
            print(f"[{s}] build")
            build_variant(a.app, s, work / s, targets)
    job = make_job(a.app, meta, styles, dist, work, a.impl, rsout, targets)
    job.signing = signing             # AppImage and Flatpak signatures go into the report
    if "appimage" in formats and not (a.appimage_runtime or os.environ.get("jfgui_APPIMAGE_RUNTIME")):
        job.runtime = build_runtime(rsout, a.profile)
    job.appimage_runtime = a.appimage_runtime
    deb = None
    if "win" in targets:
        sign_variants(a.app, styles, work, signing, a.impl)
    if "setup" in formats:
        setup = make_setup(a.app, meta, styles, dist, work, a.impl, rsout, a.profile)
        signing.windows([setup], f"{meta.get('display', a.app)} Setup")
    if "msix" in formats:
        pkg_msix.make_msix(job, signing)
    if "deb" in formats:
        deb = make_deb(a.app, meta, styles, dist, work, a.impl, rsout if a.impl == "rs" else None)
        if a.repo:
            why = codesign.apt_repo([deb], ROOT / a.repo, meta.get("publisher", "JFLib"))
            if why:
                signing.skip(why)
            else:
                signing.done.append(f"apt-Repository {a.repo} (InRelease, Release.gpg, repo.gpg)")
    appimages = {}
    if "appimage" in formats:
        for s in styles:
            appimages[s] = pkg_linux.make_appimage(job, s)
    if "rpm" in formats:
        pkg_linux.make_rpm(job)
    if "flatpak" in formats:
        pkg_linux.make_flatpak(job)
    if a.update:
        update_channel(a.app, meta, styles, work, a.update, deb, a.update_url, a.update_key, signing, a.impl, appimages)
    if set(formats) - {"setup", "deb"}:
        job.report()
    signing.report()


# the package formats and the target each one packages
FORMATS = {"setup": "win", "msix": "win", "deb": "linux", "appimage": "linux", "rpm": "linux", "flatpak": "linux"}


def formats_of(text, no_win=False, no_deb=False):
    """--formats (and --no-win / --no-deb: every Windows / Linux format dropped) -> the list"""
    names = list(FORMATS) if text.strip() == "all" else [f.strip() for f in text.split(",") if f.strip()]
    bad = [f for f in names if f not in FORMATS]
    if bad:
        raise SystemExit(f"--formats: unknown {', '.join(bad)} (known: {', '.join(FORMATS)}, all)")
    return [f for f in names if not (no_win and FORMATS[f] == "win") and not (no_deb and FORMATS[f] == "linux")]


def make_job(app, meta, styles, dist, work, impl, rsout, targets):
    """the built files for pkg_linux.py / pkg_msix.py (pkgmeta.Job)"""
    pkg = meta.get("deb_name", app.lower())
    return pkgmeta.Job(
        app=app, meta=meta, styles=styles, root=ROOT, dist=ROOT / dist, work=ROOT / work, impl=impl, pkg=pkg,
        version=meta.get("version", "0.1.0"),
        exe={s: {t: ROOT / work / s / t / exe_name(app, t) for t in targets} for s in styles},
        extra={t: [(m, ROOT / work / styles[0] / t / m) for m in app_modules(app, t, impl) + app_bins(app, t, impl)]
               for t in ("win", "linux")},
        bins={t: app_bins(app, t, impl) for t in ("win", "linux")},
        cross={"linux": cross_files(app, meta, rsout if impl == "rs" else None, "win") if "linux" in targets else [],
               "win": cross_files(app, meta, rsout if impl == "rs" else None, "linux") if "win" in targets else []},
        notices=package_notices(styles, impl), units=unit_files(pkg, meta), runtime=None, appimage_runtime=None)


def build_runtime(rsout, profile):
    """apps/appimagert, the AppImage runtime (Linux, the same profile) -> its path"""
    run([sys.executable, "build_rs.py", "app", "appimagert", "--target", "linux", *rs_mode(profile), "--out", rsout])
    return ROOT / rsout / "linux" / "appimagert"


if __name__ == "__main__":
    main()
