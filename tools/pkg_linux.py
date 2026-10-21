"""pkg_linux.py -- AppImage, RPM and Flatpak of an app (python tools/package.py APP --formats
appimage,rpm,flatpak; docs/PACKAGING.md "AppImage", "RPM", "Flatpak").  The Linux tools run in
Linux or WSL (tools/linux_task.py); a missing tool skips that step and says what is missing.

  make_appimage(job, style)    <dist>/<App>-<version>-<style>-x86_64.AppImage: the AppDir
                               (AppRun, .desktop, icon, metainfo, the program with its modules,
                               further programs and notices) as SquashFS (mksquashfs -comp zstd)
                               behind the runtime apps/appimagert (or another runtime file:
                               --appimage-runtime / jfgui_APPIMAGE_RUNTIME); appimagetool in
                               PATH makes the image instead (with the same runtime)
  make_rpm(job)                <dist>/rpm/<pkg>.spec + SOURCES/<pkg>-<version>-root.tar.gz, and
                               the .rpm by rpmbuild when it is installed
  make_flatpak(job)            <dist>/flatpak/<app id>.json + files/: the manifest, finish-args
                               from apps/NAME/PERMISSIONS; <dist>/<app id>.flatpak (a bundle)
                               by flatpak-builder when it and the runtime are installed
  finish_args(perms, extra)    the sandbox holes of a PERMISSIONS manifest -> (args, notes)
  runtime_image(elf, info)     a runtime file made an AppImage runtime: the magic "AI\\x02" at
                               byte 8, the section .upd_info (1024 bytes) with info
  elf_end(b)                   where an ELF file's data ends (an AppImage's image starts there)
  probe()                      which Linux tools are there (one call into Linux / WSL)
"""
import io
import json
import os
import re
import shlex
import shutil
import struct
import sys
import tarfile
import time
from pathlib import Path

import pkgmeta
from pkgmeta import app_id, desktop_entry, filetypes, icon, lf, metainfo, mime_package

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools.host_exec import linux_path, run_linux  # noqa: E402

FLATPAK_RUNTIME = "org.freedesktop.Platform"
FLATPAK_SDK = "org.freedesktop.Sdk"
FLATPAK_VERSION = "26.08"
UPD_INFO_SIZE = 1024

_probe = None


def probe():
    """{tool: path or ""} for the Linux tools of the formats, "flatpak_refs": the installed
    runtimes (application/branch) -- asked once per run"""
    global _probe
    if _probe is None:
        r = run_linux(["python3", linux_path(ROOT / "tools" / "linux_task.py"), "probe"])
        try:
            _probe = json.loads(r.stdout) if r.returncode == 0 else {}
        except ValueError:
            _probe = {}
        if r.returncode:
            _probe["error"] = (r.stdout + r.stderr).strip() or "Linux / WSL nicht erreichbar"
    return _probe


def linux_task(*args, timeout=1800):
    r = run_linux(["python3", linux_path(ROOT / "tools" / "linux_task.py"), *map(str, args)], timeout=timeout)
    if r.returncode:
        raise RuntimeError((r.stdout + r.stderr).strip())
    return r.stdout


# ---------------------------------------------------------------- the files of a package

def program_files(job, style, prefix):
    """{path in the package: (bytes or source Path, mode)}: the program of style as
    prefix/lib/<pkg>/<pkg>, its modules and further programs (BINS) next to it, the other
    system's agents (cross_bins), the notices in prefix/share/doc/<pkg>/"""
    pkg = job.pkg
    lib = f"{prefix}/lib/{pkg}"
    out = {f"{lib}/{pkg}": (job.exe[style]["linux"], 0o755)}
    for name, src in job.extra["linux"]:
        out[f"{lib}/{name}"] = (src, 0o755)
    for name, data in job.cross.get("linux", []):
        out[f"{lib}/{name}"] = (data, 0o755 if not name.endswith(".dll") else 0o644)
    for rel, src in job.notices.items():
        out[f"{prefix}/share/doc/{pkg}/{Path(rel).as_posix()}"] = (src, 0o644)
    return out


def desktop_files(job, prefix, aid, icon_name, wm_class):
    """the .desktop file, the hicolor icons, the metainfo and the own MIME types below prefix/share"""
    meta, app = job.meta, job.app
    out = {f"{prefix}/share/applications/{aid}.desktop":
           (lf(desktop_entry(meta, app, job.pkg, icon_name, wm_class)), 0o644)}
    for size in pkgmeta.ICON_SIZES:
        out[f"{prefix}/share/icons/hicolor/{size}x{size}/apps/{icon_name}.png"] = (icon(app, size), 0o644)
    own = [t for t in filetypes(meta) if t[3]]
    if own:
        out[f"{prefix}/share/mime/packages/{aid}.xml"] = (lf(mime_package(own)), 0o644)
    return out


def write_tree(files, root):
    """files ({path: (bytes | Path, mode)}) below root (a fresh folder)"""
    if root.exists():
        shutil.rmtree(root)
    for path, (src, _) in files.items():
        dest = root / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(src, (bytes, bytearray)):
            dest.write_bytes(src)
        else:
            shutil.copyfile(src, dest)


# ---------------------------------------------------------------- AppImage

def elf_end(b):
    """the end of an ELF64 file's data: the section header table, sections and segments"""
    if b[:4] != b"\x7fELF" or b[4] != 2 or b[5] != 1:
        raise ValueError("no ELF64 little-endian file")
    phoff, shoff = struct.unpack_from("<QQ", b, 0x20)
    phsize, phnum, shsize, shnum = struct.unpack_from("<HHHH", b, 0x36)
    end = shoff + shsize * shnum
    for i in range(phnum):
        ph = phoff + i * phsize                         # Elf64_Phdr: p_offset at 8, p_filesz at 32
        end = max(end, struct.unpack_from("<Q", b, ph + 8)[0] + struct.unpack_from("<Q", b, ph + 32)[0])
    for i in range(shnum):
        typ = struct.unpack_from("<I", b, shoff + i * shsize + 4)[0]
        off, size = struct.unpack_from("<QQ", b, shoff + i * shsize + 24)
        if typ != 8:                                    # SHT_NOBITS takes no room
            end = max(end, off + size)
    return end


def elf_sections(b):
    """{name: (file offset, size)} of an ELF64 file"""
    shoff = struct.unpack_from("<Q", b, 0x28)[0]
    shsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x3A)
    heads = [struct.unpack_from("<IIQQQQ", b, shoff + i * shsize) for i in range(shnum)]
    names = b[heads[shstrndx][4]:heads[shstrndx][4] + heads[shstrndx][5]]
    out = {}
    for name, _, _, _, off, size in heads:
        out[names[name:names.index(b"\0", name)].decode("ascii", "replace")] = (off, size)
    return out


def runtime_image(elf, info=""):
    """the runtime as an AppImage runtime: "AI\\x02" at byte 8 (e_ident padding, ignored by the
    kernel) and a section .upd_info of 1024 bytes with the update information (added at the
    end: the image still starts at elf_end)"""
    data = info.encode("utf-8")
    if len(data) >= UPD_INFO_SIZE:
        raise SystemExit(f"appimage_update: longer than {UPD_INFO_SIZE - 1} bytes")
    b = bytearray(elf)
    if elf_end(b) != len(b):
        raise SystemExit("AppImage-Runtime: Daten hinter dem ELF-Ende (das Abbild würde falsch gefunden)")
    b[8:11] = b"AI\x02"
    sec = elf_sections(b)
    if ".upd_info" in sec:
        off, size = sec[".upd_info"]
        b[off:off + size] = data.ljust(size, b"\0")[:size]
        return bytes(b)
    shoff = struct.unpack_from("<Q", b, 0x28)[0]
    shsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x3A)
    if shoff + shsize * shnum != len(b):
        raise SystemExit("AppImage-Runtime: die Abschnittstabelle steht nicht am Dateiende")
    heads = [bytearray(b[shoff + i * shsize:shoff + (i + 1) * shsize]) for i in range(shnum)]
    noff, nsize = struct.unpack_from("<QQ", heads[shstrndx], 24)
    names = bytes(b[noff:noff + nsize])
    out = bytearray(b[:shoff])
    out += b"\0" * (-len(out) % 8)
    upd = len(out)
    out += data.ljust(UPD_INFO_SIZE, b"\0")
    names_at = len(out)
    out += names + b".upd_info\0"
    out += b"\0" * (-len(out) % 8)
    struct.pack_into("<QQ", heads[shstrndx], 24, names_at, len(names) + 10)
    new_shoff = len(out)
    for h in heads:
        out += h
    # Elf64_Shdr: name, SHT_PROGBITS, no flags (not loaded), addr 0, offset, size, link, info, align 1
    out += struct.pack("<IIQQQQIIQQ", len(names), 1, 0, 0, upd, UPD_INFO_SIZE, 0, 0, 1, 0).ljust(shsize, b"\0")
    struct.pack_into("<Q", out, 0x28, new_shoff)
    struct.pack_into("<H", out, 0x3C, shnum + 1)
    return bytes(out)


def metainfo_note(job, fmt):
    """AppStream (appstreamcli validate) and Flathub want a homepage"""
    if not job.meta.get("homepage") and not getattr(job, "_homepage_said", False):
        job._homepage_said = True
        job.note(fmt, "PACKAGE homepage= fehlt: appstreamcli validate warnt (url-homepage-missing), Flathub verlangt sie")


def appdir_files(job, style):
    """the AppDir: AppRun (a link to the program), <app id>.desktop and .png at the top,
    .DirIcon, usr/bin/<pkg>, usr/lib/<pkg>/, usr/share/... -> (files, links)"""
    metainfo_note(job, "appimage")
    aid, pkg = app_id(job.meta, job.app), job.pkg
    files = program_files(job, style, "usr")
    files.update(desktop_files(job, "usr", aid, aid, pkg))
    files[f"{aid}.desktop"] = files[f"usr/share/applications/{aid}.desktop"]
    files[f"{aid}.png"] = (icon(job.app, 256), 0o644)
    files[f"usr/share/metainfo/{aid}.appdata.xml"] = (lf(metainfo(job.meta, job.app, aid, pkg)), 0o644)
    links = {"AppRun": f"usr/lib/{pkg}/{pkg}", f"usr/bin/{pkg}": f"../lib/{pkg}/{pkg}", ".DirIcon": f"{aid}.png"}
    return files, links


def appimage_runtime(job):
    """the runtime file: --appimage-runtime / jfgui_APPIMAGE_RUNTIME, else apps/appimagert as
    built (job.runtime)"""
    given = job.appimage_runtime or os.environ.get("jfgui_APPIMAGE_RUNTIME")
    src = Path(given) if given else job.runtime
    if src is None or not src.is_file():
        raise RuntimeError(f"AppImage-Runtime fehlt: {src}")
    return src


def make_appimage(job, style):
    """one AppImage of style -> its path, or None when a tool is missing (job.skip says which)"""
    files, links = appdir_files(job, style)
    work = job.work / "appimage" / style
    write_tree(files, work / "AppDir")
    where = f"nur {(work / 'AppDir').relative_to(job.root).as_posix()} geschrieben (Links: {', '.join(links)})"
    p = probe()
    if p.get("error"):
        return job.skip("appimage", f"Linux / WSL nicht erreichbar ({p['error'][:200]}); {where}")
    if not p.get("appimagetool") and not p.get("mksquashfs"):
        return job.skip("appimage", f"mksquashfs fehlt (Paket squashfs-tools) und kein appimagetool; {where}")
    rt = work / "runtime"
    rt.write_bytes(runtime_image(appimage_runtime(job).read_bytes(), job.meta.get("appimage_update", "")))
    out = job.dist / f"{job.app}-{job.version}-{style}-x86_64.AppImage"
    print(f"[linux] AppImage {style}")
    linux_task("appimage", linux_path(work / "AppDir"), linux_path(rt), linux_path(out),
               *[f"{k}={v}" for k, v in links.items()])
    job.made("appimage", out)
    return out


# ---------------------------------------------------------------- RPM

def rpm_version(v):
    """an RPM Version: no '-' (pre-releases: '~')"""
    v = v.replace("-", "~")
    if not re.fullmatch(r"[A-Za-z0-9._+~^]+", v):
        raise SystemExit(f"PACKAGE version {v!r} is no RPM version")
    return v


def rpm_tree(job):
    """{path: (bytes | Path, mode)} below / : every style as /usr/lib/<pkg>/<App>-<style> (as
    the .deb), modules, further programs, agents, notices, systemd units (/usr/lib/systemd:
    /lib is a link on RPM systems), .desktop, icons, MIME types"""
    pkg, app = job.pkg, job.app
    out = {}
    for s in job.styles:
        out[f"usr/lib/{pkg}/{app}-{s}"] = (job.exe[s]["linux"], 0o755)
    first = program_files(job, job.styles[0], "usr")
    out.update({k: v for k, v in first.items() if k != f"usr/lib/{pkg}/{pkg}"})
    for name, text in job.units:
        out[f"usr/lib/systemd/system/{name}.service"] = (lf(text), 0o644)
    out.update(desktop_files(job, "usr", pkg, pkg, None))
    return out


def tar_gz(files, mtime):
    """a .tar.gz of files ({path: (bytes | Path, mode)}) with root as owner and the folders"""
    buf = io.BytesIO()
    dirs = {p.as_posix() for f in files for p in Path(f).parents} - {".", ""}
    with tarfile.open(fileobj=buf, mode="w:gz", format=tarfile.GNU_FORMAT) as t:
        for d in sorted(dirs):
            ti = tarfile.TarInfo("./" + d)
            ti.type, ti.mode, ti.mtime, ti.uname, ti.gname = tarfile.DIRTYPE, 0o755, mtime, "root", "root"
            t.addfile(ti)
        for path, (src, mode) in sorted(files.items()):
            data = bytes(src) if isinstance(src, (bytes, bytearray)) else Path(src).read_bytes()
            ti = tarfile.TarInfo("./" + path)
            ti.size, ti.mode, ti.mtime, ti.uname, ti.gname = len(data), mode, mtime, "root", "root"
            t.addfile(ti, io.BytesIO(data))
    return buf.getvalue()


def rpm_quote(path):
    return '"/' + path.replace("%", "%%") + '"'


def rpm_text(s):
    """free text in a .spec: % starts a macro"""
    return s.replace("%", "%%")


def rpm_spec(job, tarball, files):
    """the .spec: a binary package from the prebuilt tree (Source0), the styles as alternatives
    (the default style first), BINS linked into /usr/bin, units known to systemd"""
    meta, pkg, app = job.meta, job.pkg, job.app
    default = meta.get("default_linux", job.styles[0])
    default = default if default in job.styles else job.styles[0]
    bins = [n for n in job.bins.get("linux", [])]
    units = [n for n, _ in job.units]
    alt = "%{_sbindir}/update-alternatives"
    install = "\n".join(f"{alt} --install %{{_bindir}}/{pkg} {pkg} /usr/lib/{pkg}/{app}-{s} {50 if s == default else 10}"
                        for s in job.styles)
    remove = "\n".join(f"  {alt} --remove {pkg} /usr/lib/{pkg}/{app}-{s} || :" for s in job.styles)
    own = (f"usr/lib/{pkg}", f"usr/share/doc/{pkg}")      # the folders the package owns
    dirs = sorted({p.as_posix() for f in files for p in Path(f).parents
                   if any(p.as_posix() == o or p.as_posix().startswith(o + "/") for o in own)})
    labels = ", ".join(job.styles)
    when = time.strftime("%a %b %d %Y", time.gmtime(int(os.environ.get("SOURCE_DATE_EPOCH", time.time()))))
    homepage = meta.get("homepage", "")
    desc = rpm_text(meta.get("description", meta.get("summary", app)))
    return (
        f"# {pkg}.spec -- written by tools/package.py from apps/{app}/PACKAGE (docs/PACKAGING.md \"RPM\"); do not edit\n"
        "# rpmbuild -bb --define \"_topdir $PWD\" SPECS/" + pkg + ".spec   (SOURCES/" + tarball + ")\n"
        "%global debug_package %{nil}\n"
        "%global __os_install_post %{nil}\n"
        "%global _build_id_links none\n"
        "%global _missing_build_ids_terminate_build 0\n\n"
        f"Name:           {pkg}\n"
        f"Version:        {rpm_version(meta.get('version', '0.1.0'))}\n"
        "Release:        1\n"
        f"Summary:        {rpm_text(meta.get('summary', app))}\n"
        f"License:        {meta.get('license', 'LicenseRef-proprietary')}\n"
        + (f"URL:            {homepage}\n" if homepage else "")
        + f"Source0:        {tarball}\n"
        "ExclusiveArch:  x86_64\n"
        "AutoReqProv:    no\n"
        f"Requires(post): {alt}\n"
        f"Requires(preun): {alt}\n\n"
        f"%description\n{desc}\n\n"
        f"Enthält das Programm in {len(job.styles)} Stil(en) ({labels}); voreingestellt ist {default},\n"
        f"ändern mit „sudo update-alternatives --config {pkg}“.\n\n"
        "%prep\n%setup -q -c -T\n\n"
        "%build\n\n"
        "%install\nmkdir -p %{buildroot}\ntar -xzf %{SOURCE0} -C %{buildroot}\n\n"
        f"%post\n{install}\n"
        + "".join(f"ln -sf /usr/lib/{pkg}/{b} %{{_bindir}}/{b}\n" for b in bins)
        + ("if [ -d /run/systemd/system ]; then systemctl daemon-reload || :; fi\n" if units else "")
        + "\n%preun\nif [ $1 -eq 0 ]; then\n" + remove + "\n"
        + "".join(f"  rm -f %{{_bindir}}/{b}\n" for b in bins)
        + "".join(f"  if [ -d /run/systemd/system ]; then systemctl disable --now {u} >/dev/null 2>&1 || :; fi\n" for u in units)
        + "fi\n\n"
        "%files\n"
        + "".join(f"%dir {rpm_quote(d)}\n" for d in dirs)
        + "".join(f"{rpm_quote(f)}\n" for f in sorted(files))
        + "\n%changelog\n"
        f"* {when} {rpm_text(meta.get('publisher', 'JFLib'))} <packages@localhost> - {rpm_version(meta.get('version', '0.1.0'))}-1\n"
        f"- Paket aus apps/{app}/PACKAGE gebaut\n")


def make_rpm(job):
    """the .spec and its source tarball; the .rpm when rpmbuild is installed -> its path or None"""
    files = rpm_tree(job)
    ver = rpm_version(job.version)
    tarball = f"{job.pkg}-{ver}-root.tar.gz"
    top = job.dist / "rpm"
    (top / "SPECS").mkdir(parents=True, exist_ok=True)
    (top / "SOURCES").mkdir(parents=True, exist_ok=True)
    mtime = int(os.environ.get("SOURCE_DATE_EPOCH", time.time()))
    (top / "SOURCES" / tarball).write_bytes(tar_gz(files, mtime))
    spec = top / "SPECS" / f"{job.pkg}.spec"
    spec.write_bytes(lf(rpm_spec(job, tarball, files)))
    print(f"[linux] rpm: {spec.relative_to(job.root)}")
    p = probe()
    if p.get("error") or not p.get("rpmbuild"):
        why = "Linux / WSL nicht erreichbar" if p.get("error") else "rpmbuild fehlt (Paket rpm-build bzw. rpm)"
        return job.skip("rpm", f"{why}; nur {spec.relative_to(job.root).as_posix()} und SOURCES/{tarball} geschrieben")
    out = linux_task("rpm", linux_path(spec), linux_path(top / "SOURCES" / tarball), linux_path(job.dist)).strip()
    rpm = job.dist / Path(out.splitlines()[-1]).name if out else None
    if rpm is None or not rpm.exists():
        raise RuntimeError(f"rpmbuild: kein Paket ({out})")
    job.made("rpm", rpm)
    return rpm


# ---------------------------------------------------------------- Flatpak

# the user's folders: the variable of PERMISSIONS -> the Flatpak filesystem name
FS_XDG = {"$HOME": "home", "$DESKTOP": "xdg-desktop", "$DOCUMENTS": "xdg-documents", "$DOWNLOADS": "xdg-download",
          "$PICTURES": "xdg-pictures", "$MUSIC": "xdg-music", "$VIDEOS": "xdg-videos"}
# folders that lie inside the sandbox (or are private there): no hole needed
FS_INSIDE = {"$CONFIG": "config_dir() ist im Sandkasten ~/.var/app/ID/config",
             "$APPCONFIG": "config:// liegt in ~/.var/app/ID", "$APPDATA": "data:// liegt in ~/.var/app/ID",
             "$APPCACHE": "cache:// liegt in ~/.var/app/ID", "$APPLOG": "logs:// liegt in ~/.var/app/ID",
             "$TEMP": "/tmp ist im Sandkasten privat", "$EXE": "/app ist lesbar",
             "$FONTS": "die Schriften des Systems sind im Sandkasten sichtbar"}


def fs_location(pattern):
    """a PERMISSIONS path pattern -> (Flatpak filesystem location or None, note or None)"""
    if re.match(r"^[A-Za-z]:/|^//", pattern):
        return None, None                                   # a Windows path: not for Linux
    head, _, rest = pattern.partition("/")
    lit = []
    for seg in rest.split("/") if rest else []:
        if not seg or any(c in seg for c in "*?"):
            break
        lit.append(seg)
    sub = "/".join(lit)
    if pattern == "**":
        return "host", "** (jeder Pfad): --filesystem=host"
    if head in FS_INSIDE:
        return None, f"{pattern}: kein Loch nötig ({FS_INSIDE[head]})"
    if head in FS_XDG:
        base = FS_XDG[head]
        if base == "home":
            return ("~/" + sub if sub else "home"), None
        return (base + "/" + sub if sub else base), None
    if pattern.startswith("/"):
        path = "/" + sub if sub else None
        if path is None or path == "/":
            return "host", f"{pattern}: das ganze Dateisystem (--filesystem=host)"
        return path, None
    return None, f"{pattern}: unbekanntes Muster, übergangen"


def finish_args(perms, extra=()):
    """the finish-args of a PERMISSIONS manifest (None: no manifest, the app may do everything)
    -> (args, notes).  Always: Wayland with X11 as fallback, IPC (X11 shared memory).  The
    file dialog, open_url, open_folder, printing and autostart go through portals and need
    nothing; fs.* rules become --filesystem (fs.read :ro), secrets the Secret Service on the
    session bus, net.* the network"""
    args = ["--share=ipc", "--socket=wayland", "--socket=fallback-x11"]
    notes = []
    if perms is None:
        args += ["--share=network", "--filesystem=host", "--talk-name=org.freedesktop.secrets"]
        notes.append("kein PERMISSIONS: weite Rechte (Netz, alle Dateien, Secret Service); "
                     "apps/NAME/PERMISSIONS macht sie enger (docs/PERMISSIONS.md)")
        return args + list(extra), notes
    fs = {}                                                 # location -> writable
    for key, words in perms:
        if key in ("fs.read", "fs.write", "plugins"):
            for w in words:
                loc, note = fs_location(w)
                if note:
                    notes.append(note)
                if loc:
                    fs[loc] = fs.get(loc, False) or key == "fs.write"
        elif key in ("net.connect", "net.listen"):
            if "--share=network" not in args:
                args.append("--share=network")
        elif key == "secrets":
            if "--talk-name=org.freedesktop.secrets" not in args:
                args.append("--talk-name=org.freedesktop.secrets")
        elif key == "process":
            outside = [w for w in words if not w.startswith("$EXE")]
            if outside:
                notes.append("process " + " ".join(outside) + ": Programme außerhalb des Sandkastens sind nicht "
                             "erreichbar (flatpak-spawn --host bräuchte --talk-name=org.freedesktop.Flatpak; nicht vergeben)")
        elif key == "fs.user":
            notes.append("fs.user: Dateien über die Portale (Dateidialog, Dokument-Portal für „Öffnen mit“)")
        elif key in ("url", "print", "autostart"):
            notes.append(f"{key}: über das Portal, kein Loch nötig")
    for loc in sorted(fs):
        args.append(f"--filesystem={loc}" + ("" if fs[loc] else ":ro"))
    return args + list(extra), notes


def flatpak_files(job):
    """{path below /app: (bytes | Path, mode)}"""
    metainfo_note(job, "flatpak")
    aid, pkg = app_id(job.meta, job.app), job.pkg
    style = job.default_style("linux")
    files = program_files(job, style, "")
    files = {k.lstrip("/"): v for k, v in files.items()}
    files.update({k.lstrip("/"): v for k, v in desktop_files(job, "", aid, aid, pkg).items()})
    files[f"share/metainfo/{aid}.metainfo.xml"] = (lf(metainfo(job.meta, job.app, aid, pkg)), 0o644)
    return files


def flatpak_manifest(job, files):
    """the flatpak-builder manifest (JSON): the prebuilt files as a dir source, installed with
    their modes, /app/bin/<pkg> a link to the program"""
    meta, pkg = job.meta, job.pkg
    aid = app_id(meta, job.app)
    extra = meta.get("flatpak_args", "").split()
    args, notes = finish_args(job.perms, extra)
    cmds = []
    for path, (_, mode) in sorted(files.items()):
        if not path.startswith("share/"):
            cmds.append(f"install -Dm{mode:o} {shlex.quote('files/' + path)} {shlex.quote('/app/' + path)}")
    cmds += ["mkdir -p /app/bin", f"ln -s ../lib/{pkg}/{pkg} /app/bin/{pkg}",
             "cp -r --no-preserve=mode,ownership files/share /app/"]
    runtime = meta.get("flatpak_runtime", f"{FLATPAK_RUNTIME}//{FLATPAK_VERSION}")
    name, _, branch = runtime.partition("//")
    sdk = name.replace(".Platform", ".Sdk") if name.endswith(".Platform") else FLATPAK_SDK
    man = {
        "id": aid,
        "runtime": name,
        "runtime-version": branch or FLATPAK_VERSION,
        "sdk": sdk,
        "command": pkg,
        "finish-args": args,
        "modules": [{
            "name": pkg,
            "buildsystem": "simple",
            "build-commands": cmds,
            "sources": [{"type": "dir", "path": "files", "dest": "files"}],
        }],
    }
    return man, notes


def make_flatpak(job):
    """the manifest + files/; the .flatpak bundle when flatpak-builder and the runtime are
    there -> its path or None"""
    aid = app_id(job.meta, job.app)
    d = job.dist / "flatpak"
    files = flatpak_files(job)
    write_tree(files, d / "files")
    man, notes = flatpak_manifest(job, files)
    mf = d / f"{aid}.json"
    mf.write_bytes(lf(json.dumps(man, indent=2, ensure_ascii=False) + "\n"))
    print(f"[linux] flatpak: {mf.relative_to(job.root)}")
    for n in notes:
        print(f"  [flatpak] {n}")
    job.flatpak_notes = notes
    p = probe()
    if p.get("error") or not p.get("flatpak-builder"):
        why = "Linux / WSL nicht erreichbar" if p.get("error") else "flatpak-builder fehlt"
        return job.skip("flatpak", f"{why}; nur das Manifest {mf.relative_to(job.root).as_posix()} geschrieben")
    refs = p.get("flatpak_refs", [])
    need = [f"{man['runtime']}/{man['runtime-version']}", f"{man['sdk']}/{man['runtime-version']}"]
    missing = [r for r in need if r not in refs]
    if missing:
        return job.skip("flatpak", "Runtime fehlt: " + ", ".join(missing).replace("/", "//")
                        + f" (flatpak install --user flathub …: Download); nur das Manifest geschrieben")
    out = job.dist / f"{aid}.flatpak"
    linux_task("flatpak", linux_path(d), aid, linux_path(out))
    job.made("flatpak", out)
    return out
