"""pkg_msix.py -- the MSIX package of an app (python tools/package.py APP --formats msix;
docs/PACKAGING.md "MSIX").  One package of the default Windows style: the program, its modules
and further programs, the notices, the logos and AppxManifest.xml, packed by MakeAppx.exe of the
Windows SDK and signed by tools/codesign.py (the certificate's subject must be the manifest's
Publisher).

  appx_manifest(job, style)  AppxManifest.xml: the identity (PACKAGE msix_name / app_id,
                             msix_publisher / jfgui_MSIX_PUBLISHER, version as a.b.c.0), the
                             visual elements, the file types (each extension once; MakeAppx
                             refuses reserved ones like .exe) and URL schemes of PACKAGE, a
                             StartupTask (TaskId "Autostart": plat's autostart_set switches it)
                             when the app may start at login, an execution alias (PACKAGE
                             msix_alias=), capabilities
  appinstaller(job, msix)    <App>.appinstaller for App Installer (PACKAGE appinstaller_url=,
                             the folder the .appinstaller and the .msix are published in): the
                             package's identity and address, UpdateSettings OnLaunch (every
                             start, PACKAGE appinstaller_hours=) with a prompt, background checks
  capabilities(perms)        the capabilities of a PERMISSIONS manifest (+ notes)
  find_makeappx()            MakeAppx.exe: jfgui_MAKEAPPX, PATH, the newest Windows Kit
  make_msix(job, signing)    <dist>/<App>-<version>-x64.msix (+ <App>.appinstaller) -> its path
                             or None
"""
import os
import re
import shutil
import subprocess
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr

from pkgmeta import app_id, filetypes, icon, lf, logo, schemes

ROOT = Path(__file__).resolve().parent.parent
# the StartupTask's id: lib/plat/win_autostart.rs asks StartupTask.GetAsync for it
STARTUP_TASK_ID = "Autostart"
APPINSTALLER_NS = "http://schemas.microsoft.com/appx/appinstaller/2018"
LOGOS = {"StoreLogo.png": (50, 50), "Square44x44Logo.png": (44, 44), "Square150x150Logo.png": (150, 150),
         "Wide310x150Logo.png": (310, 150)}
# URL schemes Windows keeps for itself (a package may not register them)
RESERVED_SCHEMES = {"application.manifest", "application.reference", "batfile", "blob", "cmd", "com", "comfile",
                    "craplog", "dll", "drvfile", "exefile", "explorer.exe", "fax", "file", "hlp", "hta", "htafile",
                    "http", "https", "inffile", "insfile", "jar", "javascript", "jsefile", "lnk", "mailto",
                    "ms-accountpictureprovider", "ms-appdata", "ms-appx", "ms-autoplay", "msi", "ms-excel",
                    "msisam", "ms-powerpoint", "ms-settings", "ms-word", "regfile", "res", "scrfile", "scriptletfile",
                    "shell", "shtml", "sysfile", "tel", "vbefile", "vbscript", "vbsfile", "windowsmediacenterapp",
                    "wsffile", "wshfile"}


def find_makeappx():
    """MakeAppx.exe: jfgui_MAKEAPPX, PATH, the newest x64 MakeAppx of the Windows Kits -> None"""
    env = os.environ.get("jfgui_MAKEAPPX")
    if env:
        return Path(env) if Path(env).is_file() else None
    found = shutil.which("makeappx")
    if found:
        return Path(found)
    kits = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits" / "10" / "bin"
    tools = sorted(kits.glob("*/x64/makeappx.exe"), key=lambda p: tuple(int(x) for x in p.parent.parent.name.split(".") if x.isdigit()))
    return tools[-1] if tools else None


def msix_version(v):
    """PACKAGE version -> the four numbers of an MSIX identity (the fourth is 0: the Store
    keeps it for itself)"""
    nums = re.findall(r"\d+", v.split("-")[0].split("+")[0])[:3]
    if not nums or any(int(n) > 65535 for n in nums):
        raise SystemExit(f"PACKAGE version {v!r}: no MSIX version (up to three numbers 0..65535)")
    return ".".join((nums + ["0", "0", "0"])[:3]) + ".0"


def identity_name(meta, app):
    """PACKAGE msix_name, else the app id ('_' as '-'): 3..50 of [A-Za-z0-9.-]"""
    name = meta.get("msix_name") or app_id(meta, app).replace("_", "-")
    if not re.fullmatch(r"[A-Za-z0-9.-]{3,50}", name):
        raise SystemExit(f"MSIX identity name {name!r}: 3..50 letters, digits, '.', '-' (PACKAGE msix_name=)")
    return name


def publisher(meta):
    """the Publisher (the signing certificate's subject): jfgui_MSIX_PUBLISHER, PACKAGE
    msix_publisher, else CN=<publisher>"""
    pub = os.environ.get("jfgui_MSIX_PUBLISHER") or meta.get("msix_publisher") or f"CN={meta.get('publisher', 'JFLib')}"
    if not re.fullmatch(r"(CN|O|OU|L|S|C|E|STREET|DC|SERIALNUMBER)=.+", pub):
        raise SystemExit(f"MSIX publisher {pub!r}: a distinguished name like CN=Name, O=Firm, C=DE")
    return pub


def capabilities(perms):
    """the capabilities of a PERMISSIONS manifest (None: no manifest) -> (names, notes).  A
    full-trust desktop app is not confined by them (they are shown in the Store); runFullTrust
    is always declared, PERMISSIONS stays what plat enforces.  audio.output needs none: playing
    sound is no capability in MSIX (only recording would be: microphone)"""
    if perms is None:
        return ["internetClient", "internetClientServer", "privateNetworkClientServer"], \
            ["kein PERMISSIONS: alle Netz-Fähigkeiten angegeben"]
    keys = {k for k, _ in perms}
    caps, notes = [], []
    if "net.connect" in keys:
        caps.append("internetClient")
    if "net.listen" in keys:
        caps += ["internetClientServer", "privateNetworkClientServer"]
    if "audio.output" in keys:
        notes.append("audio.output: keine Fähigkeit nötig (Wiedergabe)")
    return caps, notes


def startup_task(perms):
    """a StartupTask (start at login) when the app may switch autostart on"""
    return perms is None or any(k == "autostart" for k, _ in perms)


def appx_manifest(job, style):
    meta, app = job.meta, job.app
    display = escape(meta.get("display", app), {'"': "&quot;"})
    desc = escape(meta.get("summary", meta.get("display", app)), {'"': "&quot;"})
    exe = f"{app}.exe"
    perms = job.perms
    caps, _ = capabilities(perms)
    ext = []
    seen = {}
    for exts, _, _, _ in filetypes(meta):
        for e in exts:
            if e in seen:
                raise SystemExit(f"MSIX: .{e} is in two entries of PACKAGE filetypes= (a package declares each extension once)")
            seen[e] = True
    # reserved types (.exe, .bat, .lnk ...) and content types: MakeAppx names them itself
    # ("Applications can't register for the ... file type association")
    for exts, mime, label, _own in filetypes(meta):
        name = re.sub(r"[^a-z0-9._-]", "-", exts[0])
        ext.append('        <uap:Extension Category="windows.fileTypeAssociation">\n'
                   f'          <uap:FileTypeAssociation Name={quoteattr(name)}>\n'
                   f"            <uap:DisplayName>{escape(label)}</uap:DisplayName>\n"
                   "            <uap:SupportedFileTypes>\n"
                   + "".join(f'              <uap:FileType ContentType={quoteattr(mime)}>.{e}</uap:FileType>\n' for e in exts)
                   + "            </uap:SupportedFileTypes>\n"
                   "          </uap:FileTypeAssociation>\n"
                   "        </uap:Extension>\n")
    for s in schemes(meta):
        if s in RESERVED_SCHEMES or len(s) < 2 or len(s) > 39:
            raise SystemExit(f"MSIX: URL scheme {s!r} is reserved by Windows or has a bad length (2..39)")
        ext.append('        <uap:Extension Category="windows.protocol">\n'
                   f'          <uap:Protocol Name={quoteattr(s)}>\n'
                   f"            <uap:DisplayName>{display}</uap:DisplayName>\n"
                   "          </uap:Protocol>\n"
                   "        </uap:Extension>\n")
    if startup_task(perms):
        ext.append(f'        <desktop:Extension Category="windows.startupTask" Executable={quoteattr(exe)} '
                   'EntryPoint="Windows.FullTrustApplication">\n'
                   f'          <desktop:StartupTask TaskId="{STARTUP_TASK_ID}" Enabled="false" DisplayName="{display}"/>\n'
                   "        </desktop:Extension>\n")
    alias = meta.get("msix_alias", "")
    if alias:
        if not re.fullmatch(r"[A-Za-z0-9_.-]{1,60}\.exe", alias):
            raise SystemExit(f"MSIX: msix_alias {alias!r}: a file name ending in .exe")
        ext.append(f'        <uap3:Extension Category="windows.appExecutionAlias" Executable={quoteattr(exe)} '
                   'EntryPoint="Windows.FullTrustApplication">\n'
                   "          <uap3:AppExecutionAlias>\n"
                   f"            <desktop:ExecutionAlias Alias={quoteattr(alias)}/>\n"
                   "          </uap3:AppExecutionAlias>\n"
                   "        </uap3:Extension>\n")
    lang = meta.get("language", "de-DE")
    return ('<?xml version="1.0" encoding="utf-8"?>\n'
            f"<!-- written by tools/package.py from apps/{app}/PACKAGE (docs/PACKAGING.md \"MSIX\") -->\n"
            '<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"\n'
            '         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"\n'
            '         xmlns:uap3="http://schemas.microsoft.com/appx/manifest/uap/windows10/3"\n'
            '         xmlns:desktop="http://schemas.microsoft.com/appx/manifest/desktop/windows10"\n'
            '         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"\n'
            '         IgnorableNamespaces="uap uap3 desktop rescap">\n'
            f"  <Identity Name={quoteattr(identity_name(meta, app))} Publisher={quoteattr(publisher(meta))}\n"
            f'            Version="{msix_version(meta.get("version", "0.1.0"))}" ProcessorArchitecture="x64"/>\n'
            "  <Properties>\n"
            f"    <DisplayName>{display}</DisplayName>\n"
            f"    <PublisherDisplayName>{escape(meta.get('publisher', 'JFLib'))}</PublisherDisplayName>\n"
            "    <Logo>Assets\\StoreLogo.png</Logo>\n"
            f"    <Description>{desc}</Description>\n"
            "  </Properties>\n"
            "  <Dependencies>\n"
            '    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0"/>\n'
            "  </Dependencies>\n"
            f"  <Resources>\n    <Resource Language={quoteattr(lang)}/>\n  </Resources>\n"
            "  <Applications>\n"
            f'    <Application Id="App" Executable={quoteattr(exe)} EntryPoint="Windows.FullTrustApplication">\n'
            f'      <uap:VisualElements DisplayName="{display}" Description="{desc}" BackgroundColor="transparent"\n'
            '                          Square150x150Logo="Assets\\Square150x150Logo.png" Square44x44Logo="Assets\\Square44x44Logo.png">\n'
            '        <uap:DefaultTile Wide310x150Logo="Assets\\Wide310x150Logo.png"/>\n'
            "      </uap:VisualElements>\n"
            + ("      <Extensions>\n" + "".join(ext) + "      </Extensions>\n" if ext else "")
            + "    </Application>\n"
            "  </Applications>\n"
            "  <Capabilities>\n"
            + "".join(f'    <Capability Name="{c}"/>\n' for c in caps)
            + '    <rescap:Capability Name="runFullTrust"/>\n'
            "  </Capabilities>\n"
            "</Package>\n")


def layout(job, style, d):
    """the package's folder: <App>.exe, modules and further programs, the other system's
    agents, licenses, Assets\\*.png, AppxManifest.xml"""
    if d.exists():
        shutil.rmtree(d)
    (d / "Assets").mkdir(parents=True)
    shutil.copyfile(job.exe[style]["win"], d / f"{job.app}.exe")
    for name, src in job.extra["win"]:
        shutil.copyfile(src, d / name)
    for name, data in job.cross.get("win", []):
        (d / name).parent.mkdir(parents=True, exist_ok=True)
        (d / name).write_bytes(data)
    for rel, src in job.notices.items():
        (d / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, d / rel)
    for name, (w, h) in LOGOS.items():
        (d / "Assets" / name).write_bytes(icon(job.app, w) if w == h else logo(job.app, w, h))
    (d / "AppxManifest.xml").write_bytes(lf(appx_manifest(job, style)))


def make_msix(job, signing):
    """the .msix of the default Windows style, signed when a certificate is configured"""
    style = job.default_style("win")
    d = job.work / "msix" / style
    layout(job, style, d)
    print(f"[win] msix: {(d / 'AppxManifest.xml').relative_to(job.root)}")
    tool = find_makeappx()
    if tool is None:
        return job.skip("msix", "MakeAppx.exe nicht gefunden (Windows SDK oder jfgui_MAKEAPPX); "
                        f"nur {d.relative_to(job.root).as_posix()} geschrieben")
    out = job.dist / f"{job.app}-{job.version}-x64.msix"
    r = subprocess.run([str(tool), "pack", "/o", "/d", str(d), "/p", str(out)], capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    if r.returncode:
        raise RuntimeError(f"MakeAppx pack:\n{r.stdout}{r.stderr}")
    if signing.pe:
        signing.msix(out, publisher(job.meta))
    else:
        job.note("msix", "unsigniert: Add-AppxPackage braucht eine vertrauenswürdige Signatur "
                         "(der Store nimmt das Paket unsigniert und signiert selbst)")
    job.made("msix", out)
    write_appinstaller(job, out)
    return out


def appinstaller_hours(meta):
    """PACKAGE appinstaller_hours= (HoursBetweenUpdateChecks, 0..255; default 0: every start)"""
    h = meta.get("appinstaller_hours", "0")
    if not h.isdigit() or int(h) > 255:
        raise SystemExit(f"PACKAGE appinstaller_hours {h!r}: 0..255")
    return int(h)


def appinstaller(job, msix):
    """the .appinstaller of the package msix (PACKAGE appinstaller_url=: the folder both are
    published in): App Installer installs the package from there and, as UpdateSettings say,
    looks for a newer one at every start (OnLaunch; the user is asked, the start is not held
    up) and in the background; a lower version is never installed over a higher one"""
    meta = job.meta
    base = meta.get("appinstaller_url", "").rstrip("/")
    if not re.fullmatch(r"https?://[^\s\"<>]+", base):
        raise SystemExit(f"PACKAGE appinstaller_url {base!r}: an http(s) address of a folder")
    version = msix_version(meta.get("version", "0.1.0"))
    name = f"{job.app}.appinstaller"
    return ('<?xml version="1.0" encoding="utf-8"?>\n'
            f"<!-- written by tools/package.py from apps/{job.app}/PACKAGE (docs/PACKAGING.md \"MSIX\") -->\n"
            f'<AppInstaller xmlns="{APPINSTALLER_NS}" Version="{version}" Uri={quoteattr(base + "/" + name)}>\n'
            f"  <MainPackage Name={quoteattr(identity_name(meta, job.app))} Publisher={quoteattr(publisher(meta))}\n"
            f'               Version="{version}" ProcessorArchitecture="x64" Uri={quoteattr(base + "/" + Path(msix).name)}/>\n'
            "  <UpdateSettings>\n"
            f'    <OnLaunch HoursBetweenUpdateChecks="{appinstaller_hours(meta)}" ShowPrompt="true" UpdateBlocksActivation="false"/>\n'
            "    <AutomaticBackgroundTask/>\n"
            "  </UpdateSettings>\n"
            "</AppInstaller>\n")


def write_appinstaller(job, msix):
    """<dist>/<App>.appinstaller when PACKAGE appinstaller_url= is set, else a note"""
    if not job.meta.get("appinstaller_url"):
        job.note("msix", "PACKAGE appinstaller_url= fehlt: keine .appinstaller-Datei (Updates über App Installer); "
                         "lib/update meldet das Paket als von außen aktualisiert")
        return None
    out = job.dist / f"{job.app}.appinstaller"
    out.write_bytes(lf(appinstaller(job, msix)))
    job.made("msix", out)
    return out


def package_files(msix):
    """the file names inside an .msix (a ZIP), without the block map / signature parts"""
    with zipfile.ZipFile(msix) as z:
        return sorted(n for n in z.namelist() if n not in ("AppxBlockMap.xml", "[Content_Types].xml", "AppxSignature.p7x"))
