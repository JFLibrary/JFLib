"""codesign.py -- signing for tools/package.py: Authenticode (Windows), a signed apt repository
(Linux) and the report of what is missing (docs/PACKAGING.md, "Signing")

  authenticode()                  -> Signer or the reason it is not possible
  Signer.sign(paths)              signtool sign /fd SHA256 [/tr URL /td SHA256] for every file
  pe_signature(path) -> (offset, size) | None    the certificate table of a signed PE file
  apt_repo(debs, out)             a flat apt repository: Packages(.gz), Release, InRelease,
                                  Release.gpg, the public key as <out>/repo.gpg (in Linux/WSL)
  gpg_detach_sign(data)           an armored detached signature of data and the armored public
                                  key (the AppImage signature; in Linux/WSL)
  run_gpg_task(args)              a tools/linux_task.py action with the GPG settings (Flatpak)

Configuration (environment; nothing of it belongs in the repository):
  jfgui_SIGN_PFX / jfgui_SIGN_PFX_PASS   a code-signing certificate with its key (.pfx) and its
                                         password, or
  jfgui_SIGN_THUMBPRINT                  a certificate in the user's store (SHA-1 thumbprint;
                                         also hardware tokens and cloud HSM providers)
  jfgui_SIGN_TIMESTAMP                   an RFC 3161 time-stamp server (recommended: without a
                                         time stamp the signature ends with the certificate)
  jfgui_SIGNTOOL                         signtool.exe (default: PATH, else the newest Windows Kit)
  jfgui_GPG_KEY                          the key id / fingerprint that signs the apt repository,
                                         the AppImages (.sha256_sig) and the Flatpak repository
                                         and bundle
  jfgui_GPG_HOME                         its GnuPG home in Linux (default ~/.gnupg)
  jfgui_GPG_PASSPHRASE                   its passphrase (batch signing; else gpg-agent asks;
                                         flatpak signs through gpg-agent only: a key with a
                                         passphrase needs it cached there)
"""
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools.host_exec import linux_path, run_linux  # noqa: E402


def find_signtool():
    """signtool.exe: jfgui_SIGNTOOL, PATH, the newest x64 signtool of the Windows Kits -> None"""
    env = os.environ.get("jfgui_SIGNTOOL")
    if env:
        return Path(env) if Path(env).is_file() else None
    found = shutil.which("signtool")
    if found:
        return Path(found)
    kits = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits" / "10" / "bin"
    tools = sorted(kits.glob("*/x64/signtool.exe"), key=lambda p: tuple(int(x) for x in p.parent.parent.name.split(".") if x.isdigit()))
    return tools[-1] if tools else None


class Signer:
    def __init__(self, tool, args, what):
        self.tool, self.args, self.what = tool, args, what

    def sign(self, paths, desc=None, pe=True):
        """sign every file in place (one signtool run); pe: check that each is a PE file with
        a certificate table afterwards (an MSIX package is checked by its caller)"""
        paths = [str(p) for p in paths]
        if not paths:
            return
        cmd = [str(self.tool), "sign", "/fd", "SHA256", *self.args]
        if desc:
            cmd += ["/d", desc]
        ts = os.environ.get("jfgui_SIGN_TIMESTAMP")
        if ts:
            cmd += ["/tr", ts, "/td", "SHA256"]
        r = subprocess.run(cmd + paths, capture_output=True, text=True, encoding="utf-8", errors="replace")
        shown = [a if a != os.environ.get("jfgui_SIGN_PFX_PASS") else "***" for a in cmd]
        if r.returncode:
            raise RuntimeError(f"signtool failed ({' '.join(shown)}):\n{r.stdout}{r.stderr}")
        for p in paths if pe else []:
            if pe_signature(p) is None:
                raise RuntimeError(f"signtool reported success, but {p} has no signature")


def authenticode():
    """-> (Signer, None) or (None, why it is not possible)"""
    if os.name != "nt":
        return None, "Authenticode: nur unter Windows (signtool)"
    tool = find_signtool()
    if tool is None:
        return None, "Authenticode: signtool.exe nicht gefunden (Windows SDK installieren oder jfgui_SIGNTOOL setzen)"
    pfx, thumb = os.environ.get("jfgui_SIGN_PFX"), os.environ.get("jfgui_SIGN_THUMBPRINT")
    if pfx:
        if not Path(pfx).is_file():
            return None, f"Authenticode: jfgui_SIGN_PFX zeigt auf keine Datei ({pfx})"
        args = ["/f", pfx]
        if os.environ.get("jfgui_SIGN_PFX_PASS"):
            args += ["/p", os.environ["jfgui_SIGN_PFX_PASS"]]
        return Signer(tool, args, f"Zertifikat {Path(pfx).name}"), None
    if thumb:
        return Signer(tool, ["/sha1", thumb.replace(" ", "")], f"Zertifikat {thumb[:8]}… aus dem Zertifikatspeicher"), None
    return None, "Authenticode: kein Zertifikat (jfgui_SIGN_PFX + jfgui_SIGN_PFX_PASS oder jfgui_SIGN_THUMBPRINT setzen)"


def security_entry(b):
    """the file offset of a PE file's security directory entry (IMAGE_DIRECTORY_ENTRY_SECURITY:
    offset and size of the certificate table) -> None when b is no PE file"""
    if len(b) < 0x40 or b[:2] != b"MZ":
        return None
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    if pe + 24 + 112 + 40 > len(b) or b[pe:pe + 4] != b"PE\0\0":
        return None
    opt = pe + 24
    magic = struct.unpack_from("<H", b, opt)[0]
    if magic not in (0x10B, 0x20B):             # PE32 / PE32+
        return None
    dirs = opt + (96 if magic == 0x10B else 112)
    if struct.unpack_from("<I", b, dirs - 4)[0] < 5:
        return None
    return dirs + 32


def pe_signature(path):
    """the certificate table of a signed PE file: (file offset, size) -> None when unsigned"""
    return pe_signature_bytes(Path(path).read_bytes())


def pe_signature_bytes(b):
    """pe_signature of the bytes of a PE file"""
    e = security_entry(b)
    if e is None:
        return None
    off, size = struct.unpack_from("<II", b, e)
    return (off, size) if off and size and off + size == len(b) else None


def apt_repo(debs, out, origin="JFLib"):
    """a flat apt repository in out (deb [signed-by=…/repo.gpg] URL ./) signed with
    jfgui_GPG_KEY -> None, or why it was not signed (the unsigned repository is written anyway)"""
    out = Path(out)
    (out / "pool").mkdir(parents=True, exist_ok=True)
    for d in debs:
        shutil.copyfile(d, out / "pool" / Path(d).name)
    key = os.environ.get("jfgui_GPG_KEY", "")
    r = run_gpg_task(["repo", linux_path(out), origin, key])
    if r.returncode:
        raise RuntimeError(f"apt repository: {r.stdout}{r.stderr}")
    if not key:
        return "apt-Repository: unsigniert (jfgui_GPG_KEY setzen: Schlüssel-ID für InRelease / Release.gpg)"
    return None


def gpg_env():
    """the environment of a Linux task that signs with GPG: jfgui_GPG_HOME and
    jfgui_GPG_PASSPHRASE handed through WSL (None: nothing to hand)"""
    home = os.environ.get("jfgui_GPG_HOME")
    passphrase = os.environ.get("jfgui_GPG_PASSPHRASE")
    if not (home or passphrase):
        return None
    env = dict(os.environ)
    names = []
    for n, v in (("jfgui_GPG_HOME", home), ("jfgui_GPG_PASSPHRASE", passphrase)):
        if v:
            env.pop(n.upper(), None)          # os.environ on Windows has upper-case names
            env[n] = v                        # WSL hands them over as named here
            names.append(n)
    if os.name == "nt":
        env["WSLENV"] = ":".join([*names, *filter(None, [os.environ.get("WSLENV")])])
    return env


def run_gpg_task(args, timeout=1800):
    """python3 tools/linux_task.py ARGS in Linux / WSL with the GPG settings (gpg_env)"""
    return run_linux(["python3", linux_path(ROOT / "tools" / "linux_task.py"), *map(str, args)], env=gpg_env(), timeout=timeout)


def gpg_detach_sign(data):
    """an ASCII-armored detached signature of data by jfgui_GPG_KEY and the key's armored public
    part (the AppImage signature, pkg_linux.sign_appimage) -> (signature, key)"""
    import json
    import tempfile
    key = os.environ.get("jfgui_GPG_KEY", "")
    if not key:
        raise RuntimeError("jfgui_GPG_KEY ist nicht gesetzt")
    (ROOT / "build").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="gpgsign-", dir=ROOT / "build") as tmp:
        f = Path(tmp) / "digest"
        f.write_bytes(data)
        r = run_gpg_task(["gpgsign", linux_path(f), key])
    if r.returncode:
        raise RuntimeError(f"gpg: {(r.stdout + r.stderr).strip()[-2000:]}")
    out = json.loads(r.stdout.strip().splitlines()[-1])
    return out["sig"], out["key"]
