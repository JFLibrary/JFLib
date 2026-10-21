"""pkgmeta.py -- what every package format reads from an app (tools/package.py, pkg_linux.py,
pkg_msix.py, docs/PACKAGING.md): apps/NAME/PACKAGE, apps/NAME/PERMISSIONS, the .desktop entry,
the AppStream metainfo and the icons.

  kv_file(path)                     PACKAGE: key=value lines ('#' comments)
  filetypes(meta) / schemes(meta)   the file types and URL schemes the app opens
  mime_package(types)               shared-mime-info XML of the app's own types
  app_id(meta, app)                 the reverse-DNS id (PACKAGE app_id=; default org.jflib.<App>)
  permissions(app)                  apps/NAME/PERMISSIONS as [(key, [args])], None without one
  desktop_entry(meta, app, exe, icon, wm_class=None)   a .desktop file
  metainfo(meta, app, aid, binary)  AppStream metainfo (component type desktop-application)
  icon(app, size)                   PNG bytes: apps/NAME/res/app_<size>.png, else scaled down
                                    from the largest one there, else a plain placeholder
  logo(app, w, h)                   an icon centred on a transparent w x h canvas (wide tiles)
  release_date()                    YYYY-MM-DD (SOURCE_DATE_EPOCH or today)
  Job                               one package run: the built files, the report
"""
import datetime
import os
import re
from pathlib import Path
from xml.sax.saxutils import escape

import pngutil

ROOT = Path(__file__).resolve().parent.parent
ICON_SIZES = (48, 128, 256)


def lf(text):
    """UTF-8 bytes with LF line ends (Path.write_text would write CRLF on Windows)"""
    return text.encode("utf-8")


def kv_file(path):
    out = {}
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip()
    return out


def filetypes(meta):
    """PACKAGE filetypes= -> [(exts, mime, description, own)]: "ext[,ext..]|mime|description[|own]"
    entries separated by ';' (apps/installer/assoc.rs reads the same text from the manifest)"""
    out = []
    for entry in (e.strip() for e in meta.get("filetypes", "").split(";")):
        if not entry:
            continue
        f = [x.strip() for x in entry.split("|")]
        exts = [e.strip().lstrip(".").lower() for e in f[0].split(",")] if f else []
        ok = (3 <= len(f) <= 4 and (len(f) == 3 or f[3] == "own") and exts
              and all(re.fullmatch(r"[a-z0-9_+.-]{1,32}", e) for e in exts)
              and re.fullmatch(r"[a-z0-9.+-]+/[a-z0-9.+-]+", f[1]) and f[2] and not re.search(r'["<>&]', f[2]))
        if not ok:
            raise SystemExit(f"PACKAGE filetypes: bad entry {entry!r} (ext[,ext..]|mime/type|description[|own])")
        out.append((exts, f[1], f[2], len(f) == 4))
    return out


def mime_package(types):
    """the shared-mime-info XML of the app's own file types (filetypes() entries)"""
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            '<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">\n'
            + "".join(f'  <mime-type type="{m}">\n    <comment>{desc}</comment>\n'
                      + "".join(f'    <glob pattern="*.{e}"/>\n' for e in exts) + "  </mime-type>\n"
                      for exts, m, desc, _ in types)
            + "</mime-info>\n")


def schemes(meta):
    """PACKAGE schemes= -> the URL schemes the app opens (blanks, ',' or ';' between them)"""
    out = [s.lower() for s in re.split(r"[;, ]+", meta.get("schemes", "")) if s]
    for s in out:
        if not re.fullmatch(r"[a-z][a-z0-9+.-]{0,31}", s):
            raise SystemExit(f"PACKAGE schemes: bad scheme {s!r}")
    return out


def app_id(meta, app):
    """the reverse-DNS id of Flatpak, AppStream and the MSIX identity: PACKAGE app_id=, else
    org.jflib.<App>.  At least three parts of letters, digits and '_', none starting with a
    digit (the Flatpak / D-Bus rules; MSIX also takes them)"""
    aid = meta.get("app_id") or "org.jflib." + re.sub(r"[^A-Za-z0-9_]", "_", app)
    parts = aid.split(".")
    if len(parts) < 3 or len(aid) > 255 or not all(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", p) for p in parts):
        raise SystemExit(f"PACKAGE app_id: {aid!r} is no reverse-DNS id (org.example.App: three or more parts, "
                         "letters, digits, '_')")
    return aid


def permissions(app):
    """apps/NAME/PERMISSIONS -> [(key, [args])] in file order, None when the app has none (it
    may then do everything, docs/PERMISSIONS.md).  An ask line counts like the rule it asks for
    (the sandbox needs the same hole; the question is the app's gate); the quoted reason and
    comments are dropped"""
    f = ROOT / "apps" / app / "PERMISSIONS"
    if not f.exists():
        return None
    out = []
    for line in f.read_text(encoding="utf-8").splitlines():
        marks = [k for k in (line.find("#"), line.find('"')) if k >= 0]
        words = line[:min(marks) if marks else len(line)].split()
        if words[:1] == ["ask"]:
            words = words[1:]
        if words:
            out.append((words[0], words[1:]))
    return out


def desktop_entry(meta, app, exe, icon, wm_class=None):
    """the [Desktop Entry] of the app: exe on the Exec line with %U (schemes) or %F, the MIME
    types of filetypes= / schemes=; wm_class: StartupWMClass (JFLib's window class and Wayland
    app id are the name the program was started by)"""
    display = meta.get("display", app)
    types, urls = filetypes(meta), schemes(meta)
    mimes = list(dict.fromkeys([m for _, m, _, _ in types] + [f"x-scheme-handler/{s}" for s in urls]))
    return ("[Desktop Entry]\nType=Application\n"
            f"Name={display}\nGenericName={meta.get('generic', display)}\nComment={meta.get('summary', '')}\n"
            f"Exec={exe} {'%U' if urls else '%F'}\nIcon={icon}\nTerminal=false\nCategories={meta.get('categories', 'Utility;')}\n"
            + (f"MimeType={';'.join(mimes)};\n" if mimes else "")
            + (f"StartupWMClass={wm_class}\n" if wm_class else ""))


def release_date():
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    when = datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc) if epoch else datetime.datetime.now()
    return when.strftime("%Y-%m-%d")


def metainfo(meta, app, aid, binary):
    """AppStream metainfo of the app (Flatpak, AppImage; appstreamcli validate): name, summary,
    description, developer, the .desktop file it launches, the binary and MIME types it
    provides, OARS rating, the release of PACKAGE version="""
    display = meta.get("display", app)
    publisher = meta.get("publisher", "JFLib")
    dev_id = ".".join(aid.split(".")[:2])
    summary = meta.get("summary", display).rstrip(".")
    desc = meta.get("description", summary)
    mimes = [m for _, m, _, _ in filetypes(meta)]
    homepage = meta.get("homepage", "")
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            '<component type="desktop-application">\n'
            f"  <id>{aid}</id>\n"
            f"  <metadata_license>CC0-1.0</metadata_license>\n"
            f"  <project_license>{escape(meta.get('license', 'LicenseRef-proprietary'))}</project_license>\n"
            f"  <name>{escape(display)}</name>\n"
            f"  <summary>{escape(summary)}</summary>\n"
            f"  <description>\n    <p>{escape(desc)}</p>\n  </description>\n"
            f'  <developer id="{dev_id}">\n    <name>{escape(publisher)}</name>\n  </developer>\n'
            f'  <launchable type="desktop-id">{aid}.desktop</launchable>\n'
            + (f'  <url type="homepage">{escape(homepage)}</url>\n' if homepage else "")
            + "  <provides>\n"
            f"    <binary>{binary}</binary>\n"
            + "".join(f"    <mediatype>{m}</mediatype>\n" for m in mimes)
            + "  </provides>\n"
            '  <content_rating type="oars-1.1"/>\n'
            f'  <releases>\n    <release version="{escape(meta.get("version", "0.1.0"))}" date="{release_date()}"/>\n  </releases>\n'
            "</component>\n")


class Job:
    """one run of tools/package.py for the formats of pkg_linux.py / pkg_msix.py: app, meta
    (PACKAGE), styles, root, dist, work (absolute), pkg (deb_name), version; the built files:
    exe {style: {target: Path}}, extra {target: [(name, Path)]} (modules and further programs
    next to the program), bins {target: [name]} (the further programs), cross {package target:
    [(name, bytes)]} (the other system's agents), notices {relative Path: source}, units [(name,
    text)]; perms (default: permissions(app)); runtime / appimage_runtime (AppImage).  made /
    skip / note collect the report."""

    def __init__(self, **kw):
        self.__dict__.update(kw)
        if "perms" not in kw:
            self.perms = permissions(self.app)
        self.done, self.skipped, self.notes = [], [], []

    def default_style(self, target):
        """PACKAGE default_win / default_linux when it is built, else the first style"""
        d = self.meta.get("default_win" if target == "win" else "default_linux", self.styles[0])
        return d if d in self.styles else self.styles[0]

    def made(self, fmt, path):
        self.done.append((fmt, Path(path)))
        print(f"[{fmt}] {Path(path).relative_to(self.root)}: {Path(path).stat().st_size} bytes")

    def skip(self, fmt, why):
        self.skipped.append((fmt, why))
        print(f"  [{fmt}] nicht gebaut: {why}")
        return None

    def note(self, fmt, text):
        self.notes.append((fmt, text))
        print(f"  [{fmt}] {text}")

    def report(self):
        print("[formats] gebaut: " + (", ".join(f"{f} ({p.name})" for f, p in self.done) if self.done else "nichts"))
        for f, why in self.skipped:
            print(f"[formats] nicht gebaut: {f}: {why}")
        for f, text in self.notes:
            print(f"[formats] Hinweis {f}: {text}")


def _placeholder(size):
    """a rounded square with a vertical blue gradient (apps without res/app_*.png)"""
    px = bytearray(size * size * 4)
    for y in range(size):
        t = y / max(1, size - 1)
        r, g, b = int(30 + 40 * t), int(120 - 50 * t), int(230 - 60 * t)
        for x in range(size):
            o = (y * size + x) * 4
            px[o:o + 4] = bytes((r, g, b, 255))
    pngutil.round_corners(size, size, px, size * 0.18)
    return px


_warned = set()


def icon(app, size):
    """the icon in size x size: res/app_<size>.png, else the largest res/app_*.png scaled down,
    else a placeholder (said once per app)"""
    res = ROOT / "apps" / app / "res"
    exact = res / f"app_{size}.png"
    if exact.exists():
        return exact.read_bytes()
    have = sorted((int(m.group(1)), p) for p in res.glob("app_*.png") if (m := re.fullmatch(r"app_(\d+)\.png", p.name)))
    big = next((p for s, p in reversed(have) if s >= size), None)
    if big is not None:
        w, h, px = pngutil.read_png(big)
        return pngutil.png_bytes(size, size, pngutil.downscale(w, h, px, size, size))
    if app not in _warned:
        _warned.add(app)
        print(f"  (apps/{app}/res/app_256.png fehlt: Platzhalter-Symbol; python tools/mkicon.py {app} ...)")
    w = 256 if size <= 256 else size
    px = _placeholder(w)
    return pngutil.png_bytes(size, size, px if w == size else pngutil.downscale(w, w, px, size, size))


def logo(app, w, h):
    """the icon (h x h, at most w) centred on a transparent w x h canvas"""
    iw, ih, px = pngutil.png_decode(icon(app, min(w, h)))
    out = bytearray(w * h * 4)
    x0, y0 = (w - iw) // 2, (h - ih) // 2
    for y in range(ih):
        out[((y0 + y) * w + x0) * 4:((y0 + y) * w + x0 + iw) * 4] = px[y * iw * 4:(y + 1) * iw * 4]
    return pngutil.png_bytes(w, h, out)
