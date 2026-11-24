"""setup_check.py -- the Windows setup (lib/installer) installed and removed in test mode: on
Windows directly, elsewhere under Wine (docs/PACKAGING.md, "Windows: setup.exe")

    python tools/setup_check.py [--stub build/win/installer.exe] [--keep] [--shots DIR]

Setups are made here from the stub and a small payload: two styles (the second with translated
names and descriptions, as styles/S/INFO gives them), a file in a subfolder, a file type.  The
programs are stand-ins and never run.  ASMINST_SHELL_DIR keeps every run away from the user's
desktop, start menu and registry (shell.rs: test mode); ASMINST_ADMIN stands for the
administrator rights.

  pages      through the pages: German (the source) and English, French, Spanish, Italian,
             Dutch, Polish, Arabic, Hebrew (--lang); files, install.ini, the uninstall entry with
             its QuietUninstallString, shortcuts, the file type; the uninstaller removes it all,
             uninstall.exe and the folder too once its window has stayed open past two seconds
  one style  a payload with one style has no style page and no "Zurück"
  quiet      --quiet: the options (--style, --no-desktop, --no-start-menu, --no-uninstall-entry,
             --log), exit 2 for an unknown option, a missing value or an unknown style, 1 for a
             folder that cannot be made, the quiet uninstaller
  all users  --all-users: 740 without administrator rights, with them the computer's folders
             and keys (machine\\ in the test folder, scope=machine); through the pages without
             them the setup starts itself again (test mode: without a UAC prompt) and goes on
             when that one is done; uninstall.exe of it likewise
  preselect  (Wine only: it writes to the prefix's Program Files) an app installed for all users
             and not for the user: the options page preselects all users

--shots DIR writes the style, options and done pages in every language (for looking at them;
under Wine with this system's DejaVu Sans for Arabic and Hebrew, which Wine's fonts lack).
Build the stub first: python build_rs.py app installer --target win (Linux: lld-link on PATH --
LLVM's lld, or a link of that name to the toolchain's rust-lld).  Wine: its own prefix in
build/setup_check/wine.  Prints PASS / FAIL / SKIP; exit code 1 on failures.
"""
import argparse
import os
import shutil
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import pngutil  # noqa: E402

WORK = ROOT / "build" / "setup_check"
FAILS = []
LANGS = ["de", "en", "fr", "es", "it", "nl", "pl", "ar", "he"]
# the payload layout (docs/PACKAGING.md, lib/installer/payload.rs; tools/package.py writes it too)
PK_MANIFEST, PK_STYLE, PK_PREVIEW, PK_FILE = 1, 2, 3, 5
PF_ZLIB = 1
APP, DISPLAY = "checkapp", "Check App"
WINUI, CLASSIC = b"MZ stand-in winui " * 64, b"MZ stand-in classic " * 64
EXTRA = ("sub/dir/notes.txt", b"extra file\n")


def check(ok, what):
    print(("PASS " if ok else "FAIL ") + what)
    if not ok:
        FAILS.append(what)
    return ok


def skip(what, why):
    print(f"SKIP {what}: {why}")


class Host:
    """runs the setup: directly on Windows, under Wine elsewhere"""

    def __init__(self):
        self.wine = os.name != "nt"
        self.env = dict(os.environ)
        if self.wine:
            self.env.update(WINEPREFIX=str(WORK / "wine"), WINEDEBUG="-all", WINEDLLOVERRIDES="mscoree,mshtml=")

    def path(self, p):
        """the Windows path of p (Wine: the Z: drive)"""
        return str(p) if not self.wine else "Z:" + str(Path(p).resolve()).replace("/", "\\")

    def boot(self):
        if not self.wine:
            return True
        if not shutil.which("wine"):
            return False
        subprocess.run(["wineboot", "-i"], env=self.env, capture_output=True, timeout=300)
        return True

    def fonts(self):
        """(--shots) the system's fonts for characters the style's font lacks (a script run leaves
        them out unless jfgui_FONT_FALLBACK=1; a window takes them).  Wine has none for Arabic and
        Hebrew (Windows: Segoe UI, Arial): DejaVu Sans of this system copied into the prefix and
        named first, when there is one"""
        self.env["jfgui_FONT_FALLBACK"] = "1"
        src = [p for p in Path("/usr/share/fonts").rglob("DejaVuSans*.ttf") if "Mono" not in p.name and "Condensed" not in p.name]
        if not self.wine or not src:
            return
        dst = WORK / "wine" / "drive_c" / "windows" / "Fonts"
        dst.mkdir(parents=True, exist_ok=True)
        for p in src:
            shutil.copyfile(p, dst / p.name)
        self.env["jfgui_FALLBACK_FONTS"] = "DejaVu Sans"

    def run(self, exe, args, env=None, timeout=120):
        e = dict(self.env, **(env or {}))
        cmd = (["wine"] if self.wine else []) + [str(exe)] + [str(a) for a in args]
        r = subprocess.run(cmd, env=e, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout,
                           stdin=subprocess.DEVNULL)
        return r

    def code(self, c):
        """an exit code as the run reports it: Wine hands Linux only its low 8 bits (740 -> 228)"""
        return c & 0xFF if self.wine else c


def payload(entries):
    """entries: (kind, flags, name, label, data, usize) -> payload bytes (tools/package.py payload)"""
    head = b"AIPK" + struct.pack("<I", len(entries))
    recs, blobs = b"", b""
    base = 8 + 128 * len(entries)
    for kind, flags, name, label, data, usize in entries:
        recs += struct.pack("<IIIIQQ", kind, len(data), usize, flags, base + len(blobs), 0)
        recs += name.encode("utf-8")[:47].ljust(48, b"\0") + label.encode("utf-8")[:47].ljust(48, b"\0")
        blobs += data
    return head + recs + blobs


def preview(color):
    w, h = 480, 305
    return pngutil.png_bytes(w, h, bytes(color + (255,)) * (w * h))


def make_setup(stub, out, two_styles=True):
    """a setup of the stand-in app: styles winui (and classic, translated, the default; then a web
    browser too)"""
    man = [f"app={APP}", f"display={DISPLAY}", "version=1.2.3", "publisher=JFLib Tests",
           f"default={'classic' if two_styles else 'winui'}",
           "desc_winui=Fluent-Design: abgerundete Steuerelemente, Akzentfarbe",
           "filetypes=chk|application/x-check|Check-Datei|own", f"app_id=org.jflib.{APP}"]
    styles = [("winui", "Windows 11 (WinUI)", WINUI, (40, 120, 200))]
    if two_styles:
        # (and a web browser: the finish page's second option)
        man += ["schemes=http https", "browser=1", "desc_classic=Graue 3D-Kanten, dunkelblaue Titelleisten", "label_classic.en=Classic",
                "desc_classic.en=Grey 3D edges, dark blue title bars", "label_classic.fr=Classique",
                "desc_classic.fr=Bords 3D gris, barres de titre bleu foncé"]
        styles.append(("classic", "Klassisch", CLASSIC, (90, 110, 160)))
    entries = []
    for sid, label, exe, color in styles:
        png = preview(color)
        entries += [(PK_STYLE, PF_ZLIB, sid, label, zlib.compress(exe, 9), len(exe)), (PK_PREVIEW, 0, sid, label, png, len(png))]
    m = ("\n".join(man) + "\n").encode("utf-8")
    entries.insert(0, (PK_MANIFEST, PF_ZLIB, "manifest", "", zlib.compress(m, 9), len(m)))
    entries.append((PK_FILE, PF_ZLIB, EXTRA[0], "", zlib.compress(EXTRA[1], 9), len(EXTRA[1])))
    data = stub.read_bytes()
    pay = payload(entries)
    out.write_bytes(data + pay + b"ASMINST1" + struct.pack("<QQ", len(data), len(pay)))
    return out


def ini(path):
    """key=value lines -> dict"""
    if not path.exists():
        return {}
    return dict(line.split("=", 1) for line in path.read_text(encoding="utf-8", errors="replace").splitlines() if "=" in line)


def gone(path, seconds=20):
    """path disappears within seconds (the uninstaller's cmd deletes uninstall.exe and the folder
    once it has ended)"""
    end = time.time() + seconds
    while time.time() < end:
        if not path.exists():
            return True
        time.sleep(0.25)
    return not path.exists()


def script(d, name, lines):
    p = d / name
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return p


class Case:
    """one test folder: the setup, the program folder, the test-mode shell folder"""

    def __init__(self, host, setup, name):
        self.host, self.setup = host, setup
        self.d = WORK / name
        shutil.rmtree(self.d, ignore_errors=True)
        (self.d / "shell").mkdir(parents=True)
        self.inst = self.d / "inst"
        self.shell = self.d / "shell"

    def env(self, admin=None):
        e = {"ASMINST_SHELL_DIR": self.host.path(self.shell)}
        if admin is not None:
            e["ASMINST_ADMIN"] = "1" if admin else "0"
        return e

    def run(self, args, admin=None, setup=None):
        return self.host.run(setup or self.setup, args, self.env(admin))

    def ui(self, lines, args=(), admin=None, setup=None):
        s = script(self.d, f"s{len(list(self.d.glob('s*.uis')))}.uis", lines)
        return self.run(["--script", self.host.path(s), *args], admin, setup)

    def uninstaller(self, args, admin=None):
        return self.host.run(self.inst / "uninstall.exe", ["--uninstall", *args], self.env(admin))


def out(r):
    return f" -- exit {r.returncode}: {(r.stdout + r.stderr).strip()[-300:]}"


def t_pages(host, setup2):
    c = Case(host, setup2, "pages")
    r = c.ui(['expect "Weiter" exists', 'expect "Klassisch" checked', 'expect "Windows 11 (WinUI)" unchecked',
              'click "Weiter"', 'expect "Installieren für" exists', 'expect "Nur für mich (ohne Administratorrechte)" checked',
              'expect "Zurück" exists', 'click "Installieren"', 'wait 500', 'expect "Fertigstellen" exists',
              'expect text:"Verknüpfung auf dem Desktop" exists', 'expect text:"Eintrag im Startmenü" exists',
              f'expect "{DISPLAY} jetzt starten" unchecked', 'expect "Als Standardbrowser festlegen" checked'],
             ["--dir", host.path(c.inst)])
    if not check(r.returncode == 0, "pages (de): style -> options -> done" + ("" if r.returncode == 0 else out(r))):
        return
    exe = c.inst / f"{APP}.exe"
    check(exe.exists() and exe.read_bytes() == CLASSIC, "pages: the default style (classic) is installed")
    check((c.inst / EXTRA[0]).read_bytes() == EXTRA[1], "pages: the further file in its subfolder")
    i = ini(c.inst / "install.ini")
    check(i.get("app") == APP and i.get("style") == "classic" and i.get("reg") == "1" and "scope" not in i
          and i.get("files") == EXTRA[0].replace("/", "\\") and i.get("app_id") == f"org.jflib.{APP}",
          f"pages: install.ini {sorted(i.items())}")
    reg = ini(c.shell / "registry.txt")
    un = f"\"{host.path(c.inst)}\\uninstall.exe\" --uninstall"
    check(reg.get("DisplayName") == DISPLAY and reg.get("UninstallString") == un and reg.get("QuietUninstallString") == un + " --quiet"
          and reg.get("DisplayVersion") == "1.2.3", f"pages: the uninstall entry, quiet too {sorted(reg.items())}")
    check((c.shell / "Desktop" / f"{DISPLAY}.lnk").exists() and (c.shell / "Programs" / f"{DISPLAY}.lnk").exists(),
          "pages: shortcuts on the desktop and in the start menu")
    assoc = (c.shell / "assoc.txt").read_text(encoding="utf-8") if (c.shell / "assoc.txt").exists() else ""
    check(f"Software\\Classes\\{APP}.chk" in assoc and f"Software\\RegisteredApplications|{APP}=" in assoc,
          "pages: the file type's ProgID, the browser's registration")
    check(not (c.shell / "machine").exists(), "pages: nothing for all users")
    # the uninstaller's window stays open 3 s: uninstall.exe and the folder still go
    r = c.uninstaller(["--script", host.path(script(c.d, "un.uis", ['expect "Deinstallieren" exists', 'click "Deinstallieren"',
                                                                     'wait 500', 'expect "Schließen" exists', 'wait 3000']))])
    check(r.returncode == 0 and not exe.exists() and not (c.inst / "sub").exists() and not (c.shell / "registry.txt").exists()
          and not (c.shell / "assoc.txt").exists() and not (c.shell / "Desktop" / f"{DISPLAY}.lnk").exists(),
          "pages: the uninstaller removes the program, the subfolder, the entry, the file type, the shortcuts" + out(r))
    check(gone(c.inst), "pages: uninstall.exe and the folder go too after the uninstaller ended (window open > 2 s)")


# the texts each language's pages show (setup.tsv)
TEXTS = {
    "en": ("Next", "Install for", "Install", "Finish", "Classic"),
    "fr": ("Suivant", "Installer pour", "Installer", "Terminer", "Classique"),
    "es": ("Siguiente", "Instalar para", "Instalar", "Finalizar", "Klassisch"),
    "it": ("Avanti", "Installa per", "Installa", "Fine", "Klassisch"),
    "nl": ("Volgende", "Installeren voor", "Installeren", "Voltooien", "Klassisch"),
    "pl": ("Dalej", "Zainstaluj dla", "Zainstaluj", "Zakończ", "Klassisch"),
    "ar": ("التالي", "التثبيت لـ", "تثبيت", "إنهاء", "Klassisch"),
    "he": ("הבא", "התקנה עבור", "התקנה", "סיום", "Klassisch"),
}


def t_languages(host, setup2, shots):
    for lang in LANGS:
        c = Case(host, setup2, f"lang_{lang}")
        if lang == "de":
            nxt, head, inst, fin, classic = "Weiter", "Installieren für", "Installieren", "Fertigstellen", "Klassisch"
        else:
            nxt, head, inst, fin, classic = TEXTS[lang]
        shot = (lambda n: [f"shot {host.path(shots / f'{lang}_{n}.png')}"]) if shots else (lambda n: [])
        r = c.ui([f'expect "{nxt}" exists', f'expect "{classic}" checked', *shot("1_style"), f'click "{nxt}"',
                  f'expect "{head}" exists', *shot("2_options"), f'click "{inst}"', "wait 500", f'expect "{fin}" exists',
                  *shot("3_done")], ["--dir", host.path(c.inst), "--lang", lang])
        check(r.returncode == 0 and (c.inst / f"{APP}.exe").exists(), f"pages ({lang}): {nxt} / {head} / {inst} / {fin}" + out(r))
        r = c.uninstaller(["--quiet", "--lang", lang])
        check(r.returncode == 0, f"pages ({lang}): removed quietly" + out(r))


def t_one_style(host, setup1):
    c = Case(host, setup1, "one_style")
    r = c.ui(['expect "Weiter" missing', 'expect "Installieren für" exists', 'expect "Zurück" missing',
              'expect "Verknüpfung auf dem Desktop erstellen" unchecked', 'click "Installieren"', 'wait 500',
              'expect "Fertigstellen" exists', f'expect "{DISPLAY} jetzt starten" checked', 'expect "Als Standardbrowser festlegen" missing'],
             ["--dir", host.path(c.inst), "--launch", "--no-desktop"])
    check(r.returncode == 0 and (c.inst / f"{APP}.exe").read_bytes() == WINUI,
          "one style: no style page, no \"Zurück\", the style installed; --launch and --no-desktop preset the pages" + out(r))
    c.uninstaller(["--quiet"])


def t_quiet(host, setup2):
    c = Case(host, setup2, "quiet")
    log = c.d / "setup.log"
    r = c.run(["--quiet", "--dir", host.path(c.inst), "--style", "winui", "--no-desktop", "--log", host.path(log)])
    exe = c.inst / f"{APP}.exe"
    check(r.returncode == 0 and exe.read_bytes() == WINUI, "quiet: exit 0, --style winui installed" + out(r))
    check(not (c.shell / "Desktop" / f"{DISPLAY}.lnk").exists() and (c.shell / "Programs" / f"{DISPLAY}.lnk").exists(),
          "quiet: --no-desktop, the start menu entry made")
    text = log.read_text(encoding="utf-8") if log.exists() else ""
    check("install: checkapp 1.2.3 (style winui) for the current user" in text and "wurde in" in text and "error:" not in text,
          f"quiet: --log {text!r}")
    check(ini(c.shell / "registry.txt").get("QuietUninstallString", "").endswith("--uninstall --quiet"), "quiet: the quiet uninstall entry")
    r = c.uninstaller(["--quiet"])
    check(r.returncode == 0 and not exe.exists() and not (c.shell / "registry.txt").exists(), "quiet: uninstall.exe --uninstall --quiet" + out(r))
    check(gone(c.inst), "quiet: uninstall.exe and the folder go")

    c = Case(host, setup2, "quiet_opts")
    r = c.run(["--quiet", "--dir", host.path(c.inst), "--no-start-menu", "--no-uninstall-entry", "--lang", "en"])
    check(r.returncode == 0 and (c.shell / "Desktop" / f"{DISPLAY}.lnk").exists() and not (c.shell / "Programs" / f"{DISPLAY}.lnk").exists()
          and not (c.shell / "registry.txt").exists() and ini(c.inst / "install.ini").get("reg") == "0",
          "quiet: --no-start-menu --no-uninstall-entry, the default style" + out(r))
    check(ini(c.inst / "install.ini").get("style") == "classic", "quiet: without --style the manifest's default")
    c.uninstaller(["--quiet"])

    c = Case(host, setup2, "quiet_bad")
    log = c.d / "bad.log"
    for args, code, what in [(["--style", "nope", "--log", host.path(log)], 2, "an unknown style"),
                             (["--bogus"], 2, "an unknown option"), (["--style"], 2, "an option without its value")]:
        r = c.run(["--quiet", "--dir", host.path(c.inst), *args])
        check(r.returncode == code and not c.inst.exists(), f"quiet: {what} -> exit {code}, nothing installed" + out(r))
    text = log.read_text(encoding="utf-8") if log.exists() else ""
    check(text.startswith("error: ") and "nope" in text and "winui, classic" in text, f"quiet: the log names the styles {text!r}")
    (c.d / "blocker").write_bytes(b"a file, no folder")
    r = c.run(["--quiet", "--dir", host.path(c.d / "blocker" / "inst")])
    check(r.returncode == 1, "quiet: a folder that cannot be made -> exit 1" + out(r))
    r = c.run(["--quiet", "--uninstall"])
    check(r.returncode == 1, "quiet: --uninstall without install.ini beside it -> exit 1" + out(r))


def t_all_users(host, setup2):
    c = Case(host, setup2, "machine_quiet")
    exe = c.inst / f"{APP}.exe"
    r = c.run(["--quiet", "--all-users", "--dir", host.path(c.inst)], admin=False)
    check(r.returncode == host.code(740) and not c.inst.exists(), "all users: --quiet without administrator rights -> 740, nothing installed" + out(r))
    r = c.run(["--quiet", "--all-users", "--dir", host.path(c.inst)], admin=True)
    m = c.shell / "machine"
    check(r.returncode == 0 and exe.exists() and ini(c.inst / "install.ini").get("scope") == "machine", "all users: installed with them, scope=machine" + out(r))
    check(ini(m / "registry.txt").get("DisplayName") == DISPLAY and (m / "Desktop" / f"{DISPLAY}.lnk").exists()
          and (m / "Programs" / f"{DISPLAY}.lnk").exists() and (m / "assoc.txt").exists(),
          "all users: the computer's entry, shortcuts and file type (machine\\)")
    check(not (c.shell / "registry.txt").exists() and not (c.shell / "Desktop").exists(), "all users: nothing of the user's")
    r = c.uninstaller(["--quiet"], admin=False)
    check(r.returncode == host.code(740) and exe.exists(), "all users: removing it without administrator rights -> 740" + out(r))
    r = c.uninstaller(["--quiet"], admin=True)
    check(r.returncode == 0 and not exe.exists() and not (m / "registry.txt").exists() and not (m / "Desktop" / f"{DISPLAY}.lnk").exists(),
          "all users: removed with them" + out(r))
    gone(c.inst)

    # through the pages without the rights: the setup starts itself again (test mode: no UAC)
    c = Case(host, setup2, "machine_pages")
    exe = c.inst / f"{APP}.exe"
    m = c.shell / "machine"
    radio = "Für alle Benutzer dieses Computers (mit Administratorrechten)"
    s = script(c.d, "no.uis", ['click "Weiter"', f'click "{radio}"', 'click "Installieren"', "wait 500",
                               'expect text:"Das hat nicht geklappt" exists', 'click "Zurück"', 'expect "Installieren" exists',
                               f'expect "{radio}" checked'])
    r = host.run(setup2, ["--script", host.path(s), "--dir", host.path(c.inst)], dict(c.env(False), ASMINST_UAC="no"))
    check(r.returncode == 0 and not c.inst.exists(), "all users: a refused UAC prompt -> the error page, \"Zurück\" to the options" + out(r))
    r = c.ui(['click "Weiter"', f'click "{radio}"', f'expect "{radio}" checked', 'click "Installieren"', "wait 2000",
              'expect "Fertigstellen" exists', 'expect text:"Für alle Benutzer dieses Computers" exists',
              'expect text:"Verknüpfung auf dem Desktop" exists'], ["--dir", host.path(c.inst), "--no-start-menu"], admin=False)
    check(r.returncode == 0 and exe.exists() and ini(c.inst / "install.ini").get("scope") == "machine"
          and (m / "Desktop" / f"{DISPLAY}.lnk").exists() and not (m / "Programs" / f"{DISPLAY}.lnk").exists(),
          "all users: the pages without the rights -> the setup again with them, the choices passed on" + out(r))
    # a refused UAC prompt (test mode: ASMINST_UAC=no): the error page leads back, nothing removed
    un = script(c.d, "no.uis", ['click "Deinstallieren"', "wait 500", 'expect text:"Das hat nicht geklappt" exists',
                                'click "Zurück"', 'expect "Deinstallieren" exists'])
    r = host.run(c.inst / "uninstall.exe", ["--uninstall", "--script", host.path(un)], dict(c.env(False), ASMINST_UAC="no"))
    check(r.returncode == 0 and exe.exists(), "all users: a refused UAC prompt -> the error page, \"Zurück\", nothing removed" + out(r))
    r = c.uninstaller(["--script", host.path(script(c.d, "un.uis", ['click "Deinstallieren"', "wait 2000",
                                                                     'expect "Schließen" exists', 'expect "Zurück" missing']))],
                      admin=False)
    check(r.returncode == 0 and not exe.exists() and not (m / "registry.txt").exists(),
          "all users: uninstall.exe without the rights -> itself again with them" + out(r))
    check(gone(c.inst), "all users: uninstall.exe and the folder go")


def t_preselect(host, setup2):
    if not host.wine:
        return skip("preselect", "writes to Program Files: under Wine only")
    c = Case(host, setup2, "preselect")
    r = c.run(["--quiet", "--all-users"], admin=True)
    if not check(r.returncode == 0, "preselect: installed for all users in Program Files (Wine prefix)" + out(r)):
        return
    r = c.ui(['click "Weiter"', 'expect "Für alle Benutzer dieses Computers (mit Administratorrechten)" checked'])
    check(r.returncode == 0, "preselect: the options page preselects all users" + out(r))
    prog = WORK / "wine" / "drive_c" / "Program Files" / DISPLAY
    r = host.run(prog / "uninstall.exe", ["--uninstall", "--quiet"], c.env(True))
    check(r.returncode == 0 and gone(prog), "preselect: removed again" + out(r))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--stub", default=str(ROOT / "build" / "win" / "installer.exe"), help="the setup stub (build_rs.py app installer --target win)")
    ap.add_argument("--keep", action="store_true", help="keep build/setup_check")
    ap.add_argument("--shots", help="write the pages of every language into this folder")
    a = ap.parse_args()
    for s in (sys.stdout, sys.stderr):
        s.reconfigure(encoding="utf-8", errors="replace")
    stub = Path(a.stub)
    if not stub.exists():
        skip("setup", f"{stub} missing: python build_rs.py app installer --target win")
        return 0
    WORK.mkdir(parents=True, exist_ok=True)
    host = Host()
    if not host.boot():
        skip("setup", "not Windows and no wine")
        return 0
    shots = Path(a.shots).resolve() if a.shots else None
    if shots:
        shots.mkdir(parents=True, exist_ok=True)
        host.fonts()
    setup2 = make_setup(stub, WORK / "Check_Setup.exe")
    setup1 = make_setup(stub, WORK / "Check1_Setup.exe", two_styles=False)
    t_pages(host, setup2)
    t_languages(host, setup2, shots)
    t_one_style(host, setup1)
    t_quiet(host, setup2)
    t_all_users(host, setup2)
    t_preselect(host, setup2)
    if not a.keep:
        for d in WORK.iterdir():
            if d.name != "wine":
                shutil.rmtree(d, ignore_errors=True) if d.is_dir() else d.unlink()
    print(f"\n{len(FAILS)} failed" if FAILS else "\nall passed")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
